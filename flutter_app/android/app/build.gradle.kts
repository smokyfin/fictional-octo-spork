plugins {
    id("com.android.application")
    id("kotlin-android")
    id("dev.flutter.flutter-gradle-plugin")
}

android {
    namespace = "com.incss.ff.vpn"
    compileSdk = 34
    ndkVersion = "26.3.11579264"

    defaultConfig {
        applicationId = "com.incss.ff.vpn"
        minSdk = 26
        targetSdk = 34
        versionCode = 1
        versionName = "0.1.0"
        ndk {
            abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }

    sourceSets {
        getByName("main") {
            // cargo-ndk -o places .so files in <jniLibs>/<abi>/lib*.so
            jniLibs.srcDirs("src/main/jniLibs")
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            isShrinkResources = false
            signingConfig = signingConfigs.getByName("debug")
        }
    }
}

flutter { source = "../.." }

dependencies {
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.8.1")
}

// Convenience task: cargo-build the Rust JNI cdylib for all Android ABIs and
// stage the .so files where Gradle's `jniLibs` source-set expects them.
tasks.register("cargoNdkBuild") {
    doLast {
        val abis = listOf(
            "aarch64-linux-android" to "arm64-v8a",
            "armv7-linux-androideabi" to "armeabi-v7a",
            "x86_64-linux-android" to "x86_64",
        )
        val rustRoot = file("../../../rust")
        for ((triple, abi) in abis) {
            exec {
                workingDir = rustRoot
                commandLine("cargo", "ndk", "-t", abi, "--", "build", "--release", "-p", "ff_vpn_jni")
            }
            val src = file("../../../rust/target/$triple/release/libff_vpn_jni.so")
            val dst = file("src/main/jniLibs/$abi/libff_vpn_jni.so")
            dst.parentFile.mkdirs()
            if (src.exists()) src.copyTo(dst, overwrite = true)
        }
    }
}
