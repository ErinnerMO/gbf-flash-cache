package dev.gbfcache.flashcache;

import android.app.*;
import android.content.*;
import android.net.VpnService;
import android.os.*;
import java.util.*;

/** Android owns capture, permission, scope and TUN lifetime; the core only sees a proxy client. */
public final class ProxyService extends VpnService {
    private static volatile ProxyService instance;
    private static volatile boolean running;
    private static volatile String failure = "";
    private ParcelFileDescriptor tunnel;
    private volatile boolean stopping;
    private static final java.util.concurrent.ExecutorService transport = java.util.concurrent.Executors.newSingleThreadExecutor();
    private ResultReceiver receiver;
    private final List<AppHost.Reply> stopReplies = new ArrayList<>();
    static boolean isActive() { return instance != null; }
    static Map<String,String> status() {
        Map<String,String> result = new HashMap<>();
        result.put("systemProxy", Boolean.toString(running)); result.put("proxyWarning",failure);
        return result;
    }
    @Override public void onCreate() { super.onCreate(); instance=this; }
    @Override public int onStartCommand(Intent intent, int flags, int id) {
        if (intent == null || "stop".equals(intent.getAction())) { beginStop(); return START_NOT_STICKY; }
        ResultReceiver reply = intent.getParcelableExtra("reply");
        if (receiver != null || tunnel != null || stopping) {
            respond(reply,"系统代理正在运行或停止"); return START_NOT_STICKY;
        }
        receiver=reply; failure="";
        NotificationManager manager = getSystemService(NotificationManager.class);
        manager.createNotificationChannel(new NotificationChannel("proxy","系统代理",NotificationManager.IMPORTANCE_LOW));
        PendingIntent open=PendingIntent.getActivity(this,0,new Intent(this,MainActivity.class),PendingIntent.FLAG_IMMUTABLE);
        PendingIntent stop=PendingIntent.getService(this,2,new Intent(this,ProxyService.class).setAction("stop"),PendingIntent.FLAG_IMMUTABLE);
        startForeground(2,new Notification.Builder(this,"proxy").setContentTitle("GBF Flash Cache")
            .setContentText("系统代理已开启").setSmallIcon(R.drawable.ic_status).setContentIntent(open)
            .addAction(new Notification.Action.Builder(null,"关闭系统代理",stop).build()).setOngoing(true).build());
        AppHost.worker.execute(() -> {
            try {
                if (!"true".equals(AppHost.call(this,"status",Collections.emptyMap()).get("running")))
                    throw new IllegalStateException("请先启动缓存服务");
                Map<String,String> connection=AppHost.call(this,"init",Collections.emptyMap());
                ProxySettings settings=ProxySettings.load(this);
                settings.validateStart(this);
                // Snapshot native settings on the handle queue; network checks never hold it.
                transport.execute(() -> establish(connection,settings));
            } catch (Exception | LinkageError error) { startFailed(error); }
        });
        return START_NOT_STICKY;
    }
    private void establish(Map<String,String> connection,ProxySettings settings) {
        try {
            String udpConnection=new org.json.JSONObject(connection).toString();
            if (stopping) return;
            NativeCore.captureCheck(udpConnection);
            if (stopping) return;
            Builder builder=new Builder().setSession("GBF Flash Cache").setMtu(65535).setBlocking(false)
                .addAddress("10.254.254.1",32).addAddress("fd7a:6766:6300::1",128)
                .addRoute("0.0.0.0",0).addRoute("::",0).addDnsServer("198.18.0.1");
            if (settings.mode.equals("include")) {
                for (String app : settings.included) builder.addAllowedApplication(app);
            } else {
                builder.addDisallowedApplication(getPackageName());
                if (settings.mode.equals("exclude")) for (String app : settings.excluded) builder.addDisallowedApplication(app);
            }
            if (Build.VERSION.SDK_INT >= 29) builder.setMetered(false);
            tunnel=builder.establish();
            if (tunnel == null) throw new IllegalStateException("系统代理授权已失效");
            NativeCore.captureStart(tunnel.getFd(),Integer.parseInt(connection.getOrDefault("port","8765")),udpConnection);
            AppHost.main.post(() -> {
                if (stopping) { finishStart("启动已取消"); return; }
                running=true; finishStart(null); AppHost.main.postDelayed(check,1000);
            });
        } catch (Exception | LinkageError error) { startFailed(error); }
    }
    private void startFailed(Throwable error) {
        failure=AppHost.message(error);
        AppHost.main.post(() -> { finishStart(failure); beginStop(); });
    }
    private final Runnable check = new Runnable() {
        @Override public void run() {
            if (stopping || !running) return;
            String error=NativeCore.captureStatus();
            if (!error.isEmpty()) { failure=error; beginStop(); }
            else AppHost.main.postDelayed(this,1000);
        }
    };
    private static void respond(ResultReceiver reply,String error) {
        if (reply == null) return;
        Bundle bundle=new Bundle(); if (error != null) bundle.putString("error",error);
        reply.send(error == null ? 0 : 1,bundle);
    }
    private void finishStart(String error) { respond(receiver,error); receiver=null; }
    static void requestStop(Context context,AppHost.Reply reply) {
        ProxyService service=instance;
        if (service == null) { if (reply != null) reply.done(status(),null); return; }
        if (reply != null) service.stopReplies.add(reply);
        service.beginStop();
    }
    @Override public void onRevoke() { beginStop(); }
    private void beginStop() {
        if (stopping) return;
        // Android binds an established VPN: close its TUN before waiting for service destruction.
        stopping=true; running=false; AppHost.main.removeCallbacks(check);
        finishStart("系统代理已关闭");
        transport.execute(() -> {
            // Stop must not queue behind core status/probe calls. No core handle is used here.
            if (tunnel != null) {
                try { tunnel.close(); } catch (java.io.IOException error) { failure=AppHost.message(error); }
                tunnel=null;
            }
            try { NativeCore.captureStop(); }
            catch (Exception | LinkageError error) { failure=AppHost.message(error); }
            AppHost.main.post(() -> {
                if (instance == this) instance=null;
                for (AppHost.Reply reply : stopReplies) reply.done(status(),null);
                stopReplies.clear();
                stopForeground(STOP_FOREGROUND_REMOVE);
                stopSelf();
            });
        });
    }
    @Override public void onDestroy() { beginStop(); super.onDestroy(); }
}
