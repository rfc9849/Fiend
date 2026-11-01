plugins {
    alias(libs.plugins.fiend.android.application)
    alias(libs.plugins.fiend.android.compose)
    alias(libs.plugins.fiend.hilt)
    alias(libs.plugins.kotlin.serialization)
}

android {
    namespace = "dev.fiend.android"

    val cargoVersion = providers.fileContents(rootProject.layout.projectDirectory.file("../Cargo.toml")).asText.get()
        .lineSequence().first { it.startsWith("version = ") }.substringAfter('"').substringBefore('"')
    val (major, minor, patch) = cargoVersion.substringBefore('-').split('.').map(String::toInt)

    defaultConfig {
        applicationId = "dev.fiend.android"
        versionCode = major * 1_000_000 + minor * 1_000 + patch
        versionName = cargoVersion
    }

    val keystore = providers.environmentVariable("FIEND_ANDROID_KEYSTORE").orNull
    if (keystore != null) {
        signingConfigs.create("release") {
            storeFile = file(keystore)
            storePassword = providers.environmentVariable("FIEND_ANDROID_KEYSTORE_PASSWORD").get()
            keyAlias = providers.environmentVariable("FIEND_ANDROID_KEY_ALIAS").get()
            keyPassword = providers.environmentVariable("FIEND_ANDROID_KEY_PASSWORD").get()
        }
    }

    buildTypes {
        debug {
            applicationIdSuffix = ".debug"
        }
        release {
            signingConfig = signingConfigs.findByName("release")
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    buildFeatures {
        buildConfig = true
    }
}

dependencies {
    implementation(project(":core:model"))
    implementation(project(":core:data"))
    implementation(project(":core:engine"))
    implementation(project(":core:designsystem"))
    implementation(project(":feature:home"))
    implementation(project(":feature:profiles"))
    implementation(project(":feature:settings"))

    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.core.splashscreen)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.viewmodel.navigation3)
    implementation(libs.androidx.hilt.lifecycle.viewmodel.compose)
    implementation(libs.androidx.navigation3.runtime)
    implementation(libs.androidx.navigation3.ui)
    implementation(libs.androidx.compose.material3)
    implementation(libs.kotlinx.serialization.json)
}
