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
        versionCode = 1
        versionName = "0.0.1"
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"))
            signingConfig = signingConfigs.getByName("debug")
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
