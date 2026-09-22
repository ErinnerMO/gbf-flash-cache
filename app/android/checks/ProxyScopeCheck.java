package dev.gbfcache.flashcache;
import java.util.*;
public final class ProxyScopeCheck {
    static void rejects(Runnable operation) {
        try { operation.run(); throw new AssertionError("expected rejection"); }
        catch (IllegalArgumentException expected) {}
    }
    public static void main(String[] args) {
        Set<String> included = Set.of("browser", "both");
        Set<String> excluded = Set.of("upstream", "both");
        for (String mode : List.of("all", "include", "exclude")) ProxyScope.validate(mode,included,excluded,"gfc");
        assert ProxyScope.captures("all",included,excluded,"upstream");
        assert ProxyScope.captures("include",included,excluded,"both");
        assert !ProxyScope.captures("exclude",included,excluded,"both");
        assert !ProxyScope.captures("include",included,excluded,"other");
        assert ProxyScope.captures("exclude",included,excluded,"other");
        ProxyScope.validate("include",Set.of(),excluded,"gfc");
        assert !ProxyScope.captures("include",Set.of(),excluded,"browser");
        rejects(() -> ProxyScope.validateStart("include",Set.of(),excluded,"gfc"));
        ProxyScope.validateStart("include",included,excluded,"gfc");
        ProxyScope.validateStart("all",Set.of(),excluded,"gfc");
        ProxyScope.validateStart("exclude",Set.of(),Set.of(),"gfc");
        rejects(() -> ProxyScope.validate("invalid",included,excluded,"gfc"));
        rejects(() -> ProxyScope.validate("include",Set.of("gfc"),excluded,"gfc"));
        System.out.println("Proxy scope checks passed");
    }
}
