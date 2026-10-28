plugins {
    alias(libs.plugins.fiend.android.library)
    alias(libs.plugins.fiend.android.compose)
}

android {
    namespace = "dev.fiend.core.designsystem"
}

dependencies {
    api(libs.androidx.compose.material3)
    api(libs.androidx.compose.foundation)
    api(libs.androidx.compose.ui)
    api(libs.androidx.graphics.shapes)
    implementation(libs.androidx.core.ktx)
}
