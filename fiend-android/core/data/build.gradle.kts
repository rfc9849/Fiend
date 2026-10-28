plugins {
    alias(libs.plugins.fiend.android.library)
    alias(libs.plugins.fiend.hilt)
}

android {
    namespace = "dev.fiend.core.data"
}

dependencies {
    api(project(":core:model"))
    implementation(project(":core:engine"))
    implementation(libs.androidx.core.ktx)
    implementation(libs.kotlinx.coroutines.android)
    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
}
