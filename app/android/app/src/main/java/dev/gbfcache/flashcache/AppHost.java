package dev.gbfcache.flashcache;

import android.content.Context;
import android.os.Handler;
import android.os.Looper;
import java.io.File;
import java.nio.charset.StandardCharsets;
import java.util.Map;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/** All handle access is serialized here, including service shutdown. */
final class AppHost {
    static final ExecutorService worker = Executors.newSingleThreadExecutor();
    private static final ExecutorService appScanner = Executors.newSingleThreadExecutor();
    static final Handler main = new Handler(Looper.getMainLooper());
    private static long handle;
    static volatile boolean uiAttached;
    interface Reply { void done(Map<String,String> fields, String error); }

    static Map<String,String> call(Context context, String op, Map<String,String> args) throws Exception {
        if (op.equals("proxy_settings")) return ProxySettings.load(context).fields();
        if (op.equals("proxy_apps")) return ProxySettings.apps(context);
        if (op.equals("proxy_save")) { Map<String,String> values=ProxySettings.load(context).fields(); values.putAll(args); ProxySettings settings=new ProxySettings(values); settings.save(context); return settings.fields(); }
        if (handle == 0) {
            File home = new File(context.getNoBackupFilesDir(), "flash-cache");
            handle = NativeCore.open(home.getAbsolutePath().getBytes(StandardCharsets.UTF_8), trustRoots());
            if (handle == 0) throw new IllegalStateException("无法启动缓存核心");
        }
        // The Android host owns Keystore access; the shared app saves ciphertext atomically.
        if ((op.equals("start") || op.equals("settings")) && args.containsKey("password")) {
            args = new java.util.HashMap<>(args);
            args.put("androidProtectedPassword", Credentials.encrypt(args.get("password")));
        }
        Map<String,String> result = NativeCore.call(handle, op, args);
        if (op.equals("init")) {
            result.put("systemProxyEnabled", Boolean.toString(ProxySettings.load(context).enabled));
            String encrypted = result.remove("androidProtectedPassword");
            if (encrypted != null) {
                try { result.put("password", Credentials.decrypt(encrypted)); }
                catch (Exception error) {
                    result.put("password", "");
                    result.put("passwordWarning", "上游代理密码无法解密，请重新填写并保存");
                }
            }
        }
        if (op.equals("status")) result.putAll(ProxyService.status());
        return result;
    }
    private static byte[][] trustRoots() throws Exception {
        // AndroidCAStore includes enabled system roots and this user's installed CAs.
        java.security.KeyStore store = java.security.KeyStore.getInstance("AndroidCAStore");
        store.load(null, null);
        java.util.List<byte[]> roots = new java.util.ArrayList<>();
        for (java.util.Enumeration<String> aliases = store.aliases(); aliases.hasMoreElements();) {
            java.security.cert.Certificate certificate = store.getCertificate(aliases.nextElement());
            if (certificate != null) roots.add(certificate.getEncoded());
        }
        if (roots.isEmpty()) throw new IllegalStateException("Android 信任证书为空");
        return roots.toArray(new byte[0][]);
    }
    static void submit(Context context, String op, Map<String,String> args, Reply reply) {
        Context app = context.getApplicationContext();
        (op.equals("proxy_apps") ? appScanner : worker).execute(() -> {
            try { deliver(reply, call(app, op, args), null); }
            catch (Exception | LinkageError e) { deliver(reply, null, message(e)); }
        });
    }
    static String message(Throwable error) {
        return error.getMessage() == null ? "缓存服务操作失败" : error.getMessage();
    }
    static void deliver(Reply reply, Map<String,String> fields, String error) {
        if (reply != null) main.post(() -> reply.done(fields, error));
    }
    static void stop(Context context) throws Exception {
        if(handle != 0) call(context, "stop", java.util.Collections.emptyMap());
    }
    static void releaseIfIdle() { if(!uiAttached && !CacheService.isActive()) shutdown(); }
    private static void shutdown() {
        if (handle != 0) { long owned = handle; handle = 0; NativeCore.close(owned); }
    }
}
