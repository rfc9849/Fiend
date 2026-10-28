plugins {
    alias(libs.plugins.fiend.android.library)
    alias(libs.plugins.fiend.hilt)
    alias(libs.plugins.kotlin.serialization)
}

android {
    namespace = "dev.fiend.core.data"
    testOptions { unitTests.isIncludeAndroidResources = true }
}

ksp { arg("room.schemaLocation", "$projectDir/schemas") }

tasks.withType<Test>().configureEach {
    dependsOn(":core:engine:buildRustHost")
    systemProperty("jna.library.path", rootProject.layout.projectDirectory.dir("../target/debug").asFile.absolutePath)
}

dependencies {
    api(project(":core:model"))
    implementation(project(":core:engine"))
    implementation(libs.androidx.core.ktx)
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.kotlinx.serialization.json)
    implementation(libs.androidx.room.runtime)
    implementation(libs.androidx.datastore.preferences)
    implementation(libs.tink.android)
    ksp(libs.androidx.room.compiler)
    testImplementation(libs.robolectric)
    testImplementation(libs.androidx.room.testing)
    testRuntimeOnly("net.java.dev.jna:jna:5.19.1@jar")
    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
}
