plugins {
    alias(libs.plugins.fiend.android.feature)
}

android {
    namespace = "dev.fiend.feature.profiles"
}

dependencies {
    implementation(libs.androidx.camera.camera2)
    implementation(libs.androidx.camera.lifecycle)
    implementation(libs.androidx.camera.compose)
    implementation(libs.zxing.core)
    testImplementation(libs.junit)
}
