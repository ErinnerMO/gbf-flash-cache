package dev.gbfcache.flashcache;

import java.util.*;

/** Platform-independent policy; Android applies the resulting packages by UID. */
final class ProxyScope {
    static void validate(String mode, Set<String> included, Set<String> excluded, String own) {
        if (!Arrays.asList("all", "include", "exclude").contains(mode))
            throw new IllegalArgumentException("请选择代理应用范围");
        if (included.contains(own) || excluded.contains(own))
            throw new IllegalArgumentException("本应用自动绕过系统代理，无需加入名单");
    }
    static void validateStart(String mode, Set<String> included, Set<String> excluded, String own) {
        validate(mode, included, excluded, own);
        if (mode.equals("include") && included.isEmpty())
            throw new IllegalArgumentException("请至少选择一个包含应用");
    }
    static boolean captures(String mode, Set<String> included, Set<String> excluded, String app) {
        return mode.equals("all") || (mode.equals("include") ? included.contains(app) : !excluded.contains(app));
    }
}
