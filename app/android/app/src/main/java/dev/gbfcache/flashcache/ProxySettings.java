package dev.gbfcache.flashcache;

import android.content.*;
import android.content.pm.*;
import org.json.*;
import java.util.*;

/** Only Android capture settings live here, never shared-core settings. */
final class ProxySettings {
    final String mode;
    final boolean enabled;
    final Set<String> included, excluded;
    ProxySettings(Map<String,String> values) throws Exception {
        enabled = "true".equals(values.get("enabled"));
        mode = values.getOrDefault("mode", "all");
        included = list(values.getOrDefault("included", "[]"));
        excluded = list(values.getOrDefault("excluded", "[]"));
    }
    private static Set<String> list(String json) throws Exception {
        JSONArray array = new JSONArray(json); Set<String> result = new TreeSet<>();
        for (int i=0; i<array.length(); i++) result.add(array.getString(i));
        return result;
    }
    static ProxySettings load(Context context) throws Exception {
        Map<String,String> values = new HashMap<>();
        android.content.SharedPreferences prefs = context.getSharedPreferences("system-proxy", Context.MODE_PRIVATE);
        for (String key : Arrays.asList("mode", "included", "excluded", "enabled"))
            if (prefs.contains(key)) values.put(key, prefs.getString(key, ""));
        return new ProxySettings(values);
    }
    Map<String,String> fields() {
        Map<String,String> result = new HashMap<>();
        result.put("enabled", Boolean.toString(enabled));
        result.put("mode", mode); result.put("included", new JSONArray(included).toString());
        result.put("excluded", new JSONArray(excluded).toString());
        return result;
    }
    void save(Context context) throws Exception {
        ProxyScope.validate(mode, included, excluded, context.getPackageName());
        if (ProxyService.isActive()) throw new IllegalStateException("请先停止服务");
        android.content.SharedPreferences.Editor editor = context.getSharedPreferences("system-proxy", Context.MODE_PRIVATE).edit();
        for (Map.Entry<String,String> entry : fields().entrySet()) editor.putString(entry.getKey(),entry.getValue());
        editor.remove("upstreamApp");
        if (!editor.commit()) throw new IllegalStateException("无法保存代理设置");
    }
    void validateStart(Context context) throws Exception {
        ProxyScope.validateStart(mode, included, excluded, context.getPackageName());
        PackageManager pm = context.getPackageManager();
        Set<String> active = mode.equals("include") ? included : mode.equals("exclude") ? excluded : Collections.emptySet();
        int ownUid = context.getApplicationInfo().uid;
        for (String app : active)
            if (pm.getApplicationInfo(app,0).uid == ownUid) throw new IllegalArgumentException("本应用必须绕过系统代理");
    }
    static Map<String,String> apps(Context context) throws Exception {
        PackageManager pm = context.getPackageManager();
        List<JSONObject> apps = new ArrayList<>();
        int ownUid=context.getApplicationInfo().uid;
        for (ApplicationInfo app : pm.getInstalledApplications(0)) {
            if (app.uid == ownUid || pm.checkPermission("android.permission.INTERNET", app.packageName) != PackageManager.PERMISSION_GRANTED) continue;
            apps.add(new JSONObject().put("package",app.packageName).put("label",pm.getApplicationLabel(app).toString()));
        }
        apps.sort(Comparator.comparing(a -> a.optString("label"), String.CASE_INSENSITIVE_ORDER));
        JSONArray list = new JSONArray(apps);
        return Collections.singletonMap("apps",list.toString());
    }
}
