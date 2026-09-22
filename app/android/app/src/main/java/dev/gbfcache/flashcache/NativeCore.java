package dev.gbfcache.flashcache;

import org.json.JSONObject;
import java.nio.charset.StandardCharsets;
import java.util.HashMap;
import java.util.Map;

/** JNI transport only. AppHost owns paths, execution and handle lifetime. */
final class NativeCore {
    static { System.loadLibrary("gbf_flash_cache_app"); }
    static native void captureCheck(String connection);
    static native void captureStart(int fd, int port, String connection);
    static native void captureStop();
    static native String captureStatus();
    static native long open(byte[] path, byte[][] roots);
    private static native byte[] command(long handle, byte[] json);
    static native byte[] close(long handle);
    static Map<String,String> call(long handle, String op, Map<String,String> args) throws Exception {
        JSONObject input = new JSONObject().put("op", op).put("args", new JSONObject(args));
        JSONObject result = new JSONObject(new String(command(handle, input.toString().getBytes(StandardCharsets.UTF_8)), StandardCharsets.UTF_8));
        if (!result.getBoolean("ok")) throw new IllegalStateException(result.getString("error"));
        JSONObject values = result.getJSONObject("fields");
        Map<String,String> fields = new HashMap<>();
        for (java.util.Iterator<String> keys = values.keys(); keys.hasNext();) {
            String key = keys.next(); fields.put(key, values.getString(key));
        }
        return fields;
    }
}
