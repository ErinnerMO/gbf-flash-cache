plugins {
    id("com.android.application")
    id("dev.flutter.flutter-gradle-plugin")
}

android {
    namespace = "dev.gbfcache.flashcache"
    compileSdk = flutter.compileSdkVersion
    ndkVersion = flutter.ndkVersion
    ndkPath = System.getenv("ANDROID_NDK_HOME")

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    defaultConfig {
        applicationId = "com.dena.skyleap.gfc"
        minSdk = 26
        targetSdk = flutter.targetSdkVersion
        versionCode = flutter.versionCode
        versionName = flutter.versionName
    }

    sourceSets.getByName("main").jniLibs.srcDir("src/main/jniLibs")
    signingConfigs {
        create("release") {
            storeFile = file(System.getenv("GBF_ANDROID_KEYSTORE") ?: "missing-release-key")
            storePassword = System.getenv("GBF_ANDROID_KEYSTORE_PASSWORD")
            keyAlias = "gbf-flash-cache"
            keyPassword = System.getenv("GBF_ANDROID_KEYSTORE_PASSWORD")
        }
    }
    buildTypes {
        release {
            signingConfig = signingConfigs.getByName("release")
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }
}

flutter {
    source = "../.."
}
