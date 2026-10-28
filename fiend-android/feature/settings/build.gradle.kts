plugins {
    alias(libs.plugins.fiend.android.feature)
}

android {
    namespace = "dev.fiend.feature.settings"
}

dependencies {
    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
    testImplementation("app.cash.turbine:turbine:1.2.1")
}
