plugins {
    id("com.android.application")
}

android {
    namespace = "app.deskstats"
    compileSdk = 34

    defaultConfig {
        applicationId = "app.deskstats"
        // Android 2.3: the code calls API 9 (String.format with a locale, Charset) without guards
        // and guards everything newer by hand.
        minSdk = 9
        targetSdk = 34
        versionCode = 4
        versionName = "0.4.0"
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"))
            // A personal sideloaded build: the debug key of this machine is enough (see README).
            signingConfig = signingConfigs.getByName("debug")
            vcsInfo.include = false
        }
    }

    // Kotlin's metadata for reflection is never used here; it only makes the APK larger.
    packaging { resources { excludes += setOf("kotlin/**", "DebugProbesKt.bin") } }

    lint {
        // Known notices about newer versions of Gradle and of the SDK live in the baseline, so
        // the checks stay on and a newer Android Gradle Plugin or a new notice is still reported.
        baseline = file("lint-baseline.xml")
        // Deliberate, for a sideloaded desk panel: ConfigActivity must have the settings on disk
        // before it hands over, hence commit(); cleartext to the computer by design; a fixed
        // landscape panel; backup off; SDK 34 is the newest platform installed here.
        disable += setOf(
            "ApplySharedPref",
            "InsecureBaseConfiguration",
            "UnusedAttribute",
            "DiscouragedApi",
            "DataExtractionRules",
            "OldTargetApi",
            // The views are made in code, never inflated; the screen is drawn on a canvas and
            // describes itself to TalkBack with contentDescription.
            "ViewConstructor",
            "ClickableViewAccessibility",
        )
    }
}
