plugins {
    id("com.android.application")
}

android {
    namespace = "app.deskstats"
    compileSdk = 34

    defaultConfig {
        applicationId = "app.deskstats"
        minSdk = 9
        targetSdk = 34
        // One version for the server, the panel and the app: the tag of a release, or server/Cargo.toml for a build
        // by hand. The app compares it with the latest release to offer an update.
        val version = System.getenv("DESKSTATS_VERSION")?.removePrefix("v")?.takeIf { it.isNotEmpty() }
            ?: Regex("(?m)^version = \"(.+)\"").find(rootProject.file("../server/Cargo.toml").readText())!!.groupValues[1]
        val (major, minor, patch) = (version.split(".").map { it.takeWhile(Char::isDigit).toIntOrNull() ?: 0 } + listOf(0, 0, 0)).take(3)
        versionCode = major * 10_000 + minor * 100 + patch
        versionName = version
    }

    // The release key comes from the environment (CI keeps it in secrets); without it the build is signed with the
    // debug key, which is fine for trying a build but cannot update an installed release.
    val keystore = System.getenv("DESKSTATS_KEYSTORE")?.takeIf { it.isNotEmpty() }
    signingConfigs {
        if (keystore != null) {
            create("release") {
                storeFile = file(keystore)
                storePassword = System.getenv("DESKSTATS_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("DESKSTATS_KEY_ALIAS")
                keyPassword = System.getenv("DESKSTATS_KEY_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"))
            signingConfig = signingConfigs.getByName(if (keystore != null) "release" else "debug")
            vcsInfo.include = false
        }
    }

    bundle { language { enableSplit = false } }

    packaging { resources { excludes += setOf("kotlin/**", "DebugProbesKt.bin") } }

    lint {
        baseline = file("lint-baseline.xml")
        disable += setOf(
            "ApplySharedPref",
            "InsecureBaseConfiguration",
            "UnusedAttribute",
            "DiscouragedApi",
            "DataExtractionRules",
            "OldTargetApi",
            "ViewConstructor",
            "ClickableViewAccessibility",
        )
    }
}
