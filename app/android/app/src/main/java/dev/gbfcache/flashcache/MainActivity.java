package dev.gbfcache.flashcache;

import android.app.Activity;
import android.content.*;
import android.net.Uri;
import android.os.*;
import android.provider.Settings;
import io.flutter.embedding.android.FlutterActivity;
import io.flutter.embedding.engine.FlutterEngine;
import io.flutter.plugin.common.MethodChannel;
import org.json.JSONObject;
import java.io.*;
import java.util.*;

public final class MainActivity extends FlutterActivity {
    private static final int EXPORT=42, CERTIFICATE=43, PROXY=45;
    private MethodChannel.Result pending;
    private boolean certificateExport;
    @Override public void onCreate(Bundle state) { super.onCreate(state); AppHost.uiAttached = true; }
    @Override public void configureFlutterEngine(FlutterEngine engine) {
        super.configureFlutterEngine(engine);
        new MethodChannel(engine.getDartExecutor().getBinaryMessenger(), "gbf/core").setMethodCallHandler((call, result) -> {
            Map<String,String> args = call.arguments == null ? Collections.emptyMap() : new HashMap<>((Map<String,String>)call.arguments);
            AppHost.Reply reply = (fields,error) -> { if(error==null) result.success(fields); else result.error("core",error,null); };
            try {
                switch(call.method) {
                    case "proxy_start":
                        if (pending != null) throw new IllegalStateException("请先完成当前操作");
                        requestProxy(result);
                        break;
                    case "proxy_stop": ProxyService.requestStop(this,reply); break;
                    case "license":
                        try (InputStream input=getAssets().open("gfc-license.txt"); ByteArrayOutputStream output=new ByteArrayOutputStream()) {
                            byte[] buffer=new byte[16384]; int n;
                            while((n=input.read(buffer))!=-1) output.write(buffer,0,n);
                            result.success(Collections.singletonMap("text",output.toString("UTF-8")));
                        }
                        break;
                    case "start":
                        if(pending!=null) throw new IllegalStateException("请先完成当前操作");
                        start(args,result);
                        break;
                    case "stop": CacheService.requestStop(this,reply); break;
                    case "export": case "ca":
                        if(pending!=null) throw new IllegalStateException("请先完成当前操作");
                        pending=result; certificateExport=call.method.equals("ca");
                        Intent save = new Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE)
                            .setType(certificateExport ? "application/x-x509-ca-cert" : "application/zip")
                            .putExtra(Intent.EXTRA_TITLE, certificateExport ? "gbf-flash-cache.cer" : "gbf-logs-"+System.currentTimeMillis()+".zip");
                        startActivityForResult(save,certificateExport ? CERTIFICATE : EXPORT);
                        break;
                    case "certificateSettings":
                        startActivity(new Intent(Settings.ACTION_SECURITY_SETTINGS)); result.success(Collections.emptyMap()); break;
                    default: AppHost.submit(this,call.method,args,reply);
                }
            } catch(Exception | LinkageError error) {
                if(pending==result){pending=null;}
                result.error("platform",AppHost.message(error),null);
            }
        });
    }
    private void requestProxy(MethodChannel.Result result) {
        pending=result;
        AppHost.worker.execute(() -> {
            String failure=null;
            try {
                if (!"true".equals(AppHost.call(this,"status",Collections.emptyMap()).get("running")))
                    throw new IllegalStateException("请先启动缓存服务");
                ProxySettings.load(this).validateStart(this);
            } catch (Exception | LinkageError error) { failure=AppHost.message(error); }
            final String error=failure;
            AppHost.main.post(() -> {
                if (pending != result) return;
                try {
                    if (error != null) throw new IllegalStateException(error);
                    Intent permission=android.net.VpnService.prepare(this);
                    if (permission != null) startActivityForResult(permission,PROXY);
                    else { pending=null; startProxy(result); }
                } catch (Exception errorValue) {
                    pending=null; result.error("proxy",AppHost.message(errorValue),null);
                }
            });
        });
    }
    private void startProxy(MethodChannel.Result result) {
        ResultReceiver reply=new ResultReceiver(new Handler(Looper.getMainLooper())) {
            @Override protected void onReceiveResult(int code,Bundle bundle) {
                if (code == 0) result.success(ProxyService.status());
                else result.error("proxy",bundle.getString("error"),null);
            }
        };
        startForegroundService(new Intent(this,ProxyService.class).setAction("start").putExtra("reply",reply));
    }
    private void start(Map<String,String> args, MethodChannel.Result result) {
        ResultReceiver reply = new ResultReceiver(new Handler(Looper.getMainLooper())) {
            @Override protected void onReceiveResult(int code, Bundle bundle) {
                if(code!=0){result.error("start",bundle.getString("error"),null);return;}
                try {
                    JSONObject values=new JSONObject(bundle.getString("fields"));
                    Map<String,String> fields=new HashMap<>();
                    for(Iterator<String> keys=values.keys();keys.hasNext();){String key=keys.next();fields.put(key,values.getString(key));}
                    result.success(fields);
                } catch(Exception error){result.error("start",AppHost.message(error),null);}
            }
        };
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) != android.content.pm.PackageManager.PERMISSION_GRANTED) {
            requestPermissions(new String[]{android.Manifest.permission.POST_NOTIFICATIONS}, 44);
        }
        startForegroundService(new Intent(this,CacheService.class).setAction("start")
            .putExtra("args",new JSONObject(args).toString()).putExtra("reply",reply));
    }
    @Override protected void onActivityResult(int request,int code,Intent data) {
        super.onActivityResult(request,code,data);
        if (request == PROXY) {
            MethodChannel.Result result=pending; pending=null;
            if (result != null) {
                if (code == Activity.RESULT_OK) {
                    try { startProxy(result); } catch (Exception error) { result.error("proxy",AppHost.message(error),null); }
                } else result.error("cancelled","未开启系统代理",null);
            }
            return;
        }
        if(request!=EXPORT && request!=CERTIFICATE)return;
        MethodChannel.Result result=pending;pending=null;
        if(result==null)return;
        if(code!=Activity.RESULT_OK){result.error("cancelled","操作已取消",null);return;}
        Uri uri=data==null?null:data.getData();
        if(uri==null){result.error("export","没有选择保存位置",null);return;}
        boolean ca=request==CERTIFICATE;
        Context app=getApplicationContext();
        AppHost.worker.execute(() -> {
            File temporary=null;
            try {
                File source;
                if(ca) source=new File(AppHost.call(app,"ca",Collections.emptyMap()).get("path"));
                else {
                    temporary=File.createTempFile("gbf-logs-",".zip",getCacheDir());
                    AppHost.call(app,"export",Collections.singletonMap("path",temporary.getAbsolutePath()));source=temporary;
                }
                try(InputStream input=new FileInputStream(source);OutputStream output=getContentResolver().openOutputStream(uri,"wt")){
                    if(output==null)throw new IOException("无法打开保存位置");
                    byte[] bytes=new byte[16384];int n;while((n=input.read(bytes))!=-1)output.write(bytes,0,n);
                }
                AppHost.main.post(() -> result.success(Collections.singletonMap("saved","true")));
            }catch(Exception error){AppHost.main.post(() -> result.error("export",AppHost.message(error),null));}
            finally{if(temporary!=null)temporary.delete();}
        });
    }
    @Override public void onDestroy() {
        if(pending!=null){pending.error("cancelled","界面已关闭",null);pending=null;}
        AppHost.uiAttached=false;
        AppHost.worker.execute(AppHost::releaseIfIdle);
        super.onDestroy();
    }
}
