package dev.gbfcache.flashcache;

import android.app.*;
import android.content.*;
import android.os.*;
import org.json.JSONObject;
import java.util.*;

public final class CacheService extends Service {
    private static volatile CacheService instance;
    static boolean isActive() { return instance != null; }
    private static final String CHANNEL = "cache";
    private volatile boolean stopping;
    private boolean starting;
    private final List<AppHost.Reply> stopReplies = new ArrayList<>();
    @Override public IBinder onBind(Intent intent) { return null; }
    @Override public void onCreate() { super.onCreate(); instance = this; }
    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent == null || "stop".equals(intent.getAction())) { requestStop(this,null); return START_NOT_STICKY; }
        ResultReceiver receiver = intent.getParcelableExtra("reply");
        if (starting || stopping) { reply(receiver, null, "服务正在运行或停止"); return START_NOT_STICKY; }
        starting = true;
        NotificationManager notifications = getSystemService(NotificationManager.class);
        notifications.createNotificationChannel(new NotificationChannel(CHANNEL, "缓存服务", NotificationManager.IMPORTANCE_LOW));
        PendingIntent open = PendingIntent.getActivity(this, 0, new Intent(this, MainActivity.class), PendingIntent.FLAG_IMMUTABLE);
        PendingIntent stop = PendingIntent.getService(this, 1, new Intent(this, CacheService.class).setAction("stop"), PendingIntent.FLAG_IMMUTABLE);
        startForeground(1, new Notification.Builder(this, CHANNEL).setContentTitle("GBF Flash Cache")
            .setContentText("缓存服务运行中").setSmallIcon(R.drawable.ic_status)
            .setContentIntent(open).addAction(new Notification.Action.Builder(null, "停止", stop).build()).setOngoing(true).build());
        String json = intent.getStringExtra("args");
        AppHost.worker.execute(() -> {
            try {
                JSONObject values = new JSONObject(json);
                Map<String,String> args = new HashMap<>();
                for (Iterator<String> keys = values.keys(); keys.hasNext();) { String key = keys.next(); args.put(key, values.getString(key)); }
                Map<String,String> result = AppHost.call(this, "start", args);
                if (stopping) throw new IllegalStateException("启动已取消");
                reply(receiver, result, null);
            } catch (Exception | LinkageError e) {
                try { disposeCore(); } catch(Exception ignored) {}
                reply(receiver, null, AppHost.message(e));
                AppHost.main.post(this::stopSelf);
            }
        });
        return START_NOT_STICKY;
    }
    static void requestStop(Context context, AppHost.Reply reply) {
        if (ProxyService.isActive()) {
            ProxyService.requestStop(context,(fields,error) -> requestStop(context,reply));
            return;
        }
        CacheService service = instance;
        if (service == null) { AppHost.submit(context, "stop", Collections.emptyMap(), reply); return; }
        // Called on the main thread. Completion follows destruction and core shutdown.
        if (reply != null) service.stopReplies.add(reply);
        service.stopping = true;
        service.stopSelf();
    }
    private void disposeCore() throws Exception {
        NativeCore.captureStop();
        AppHost.stop(this);
    }
    private static void reply(ResultReceiver receiver, Map<String,String> fields, String error) {
        if (receiver == null) return;
        Bundle bundle = new Bundle();
        if (error != null) bundle.putString("error", error);
        else bundle.putString("fields", new JSONObject(fields).toString());
        receiver.send(error == null ? 0 : 1, bundle);
    }
    @Override public void onDestroy() {
        stopping = true;
        ProxyService.requestStop(this, (fields, proxyError) -> AppHost.worker.execute(() -> {
            String failure = null;
            try { disposeCore(); } catch (Exception | LinkageError e) { failure = AppHost.message(e); }
            final String error = failure;
            AppHost.main.post(() -> {
                if (instance == this) instance = null;
                for (AppHost.Reply reply : stopReplies) {
                    reply.done(error == null ? Collections.singletonMap("running", "false") : null, error);
                }
                stopReplies.clear();
                AppHost.worker.execute(AppHost::releaseIfIdle);
            });
        }));
        super.onDestroy();
    }
}
