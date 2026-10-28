import org.gradle.api.file.DirectoryProperty
import org.gradle.api.tasks.OutputDirectory

plugins {
    alias(libs.plugins.fiend.android.library)
}

abstract class RustExecTask : Exec() {
    @get:OutputDirectory
    abstract val outputDirectory: DirectoryProperty
}

val workspace = rootProject.layout.projectDirectory.dir("..")
val rustInputs = files(
    workspace.file("Cargo.toml"), workspace.file("Cargo.lock"),
    fileTree(workspace.dir("fiend-core")) { include("Cargo.toml", "src/**", "testdata/template.json") },
    fileTree(workspace.dir("fiend-tun")) { include("Cargo.toml", "src/**") },
    fileTree(workspace.dir("fiend-mobile")) { include("Cargo.toml", "src/**", "uniffi.toml") },
)
val generatedBindings = layout.buildDirectory.dir("generated/uniffi")
val hostLibrary = workspace.file("target/debug/" + when {
    System.getProperty("os.name").startsWith("Mac") -> "libfiend_mobile.dylib"
    System.getProperty("os.name").startsWith("Windows") -> "fiend_mobile.dll"
    else -> "libfiend_mobile.so"
})

android {
    namespace = "dev.fiend.core.engine"
    ndkVersion = "30.0.16248370"
    defaultConfig {
        ndk { abiFilters += setOf("arm64-v8a", "armeabi-v7a", "x86_64") }
        consumerProguardFiles("consumer-rules.pro")
    }
}

val buildRustHost = tasks.register<Exec>("buildRustHost") {
    workingDir(workspace)
    inputs.files(rustInputs)
    outputs.file(hostLibrary)
    commandLine("cargo", "build", "-p", "fiend-mobile", "--lib", "--locked")
}

val generateRustBindings = tasks.register<RustExecTask>("generateRustBindings") {
    dependsOn(buildRustHost)
    workingDir(workspace)
    inputs.files(rustInputs)
    outputDirectory.set(generatedBindings)
    commandLine("cargo", "run", "-p", "fiend-mobile", "--features", "bindgen", "--bin", "uniffi-bindgen", "--locked", "--",
        "generate", "--library", hostLibrary.asFile.absolutePath, "--language", "kotlin", "--out-dir", generatedBindings.get().asFile.absolutePath, "--no-format")
}

val ndkPath = androidComponents.sdkComponents.ndkDirectory.get().asFile.absolutePath
val rustBuilds = listOf("debug", "release").associateWith { variant ->
    tasks.register<RustExecTask>("buildRust${variant.replaceFirstChar(Char::uppercaseChar)}") {
        workingDir(workspace)
        inputs.files(rustInputs)
        val destination = layout.buildDirectory.dir("rust/$variant")
        outputDirectory.set(destination)
        environment("ANDROID_NDK_HOME", ndkPath)
        commandLine(buildList {
            addAll(listOf("cargo", "ndk", "-t", "arm64-v8a", "-t", "armeabi-v7a", "-t", "x86_64", "--platform", "26", "-o", destination.get().asFile.absolutePath,
                "build", "-p", "fiend-mobile", "--lib", "--locked"))
            if (variant == "release") add("--release")
        })
    }
}

androidComponents.onVariants { variant ->
    variant.sources.kotlin?.addGeneratedSourceDirectory(generateRustBindings, RustExecTask::outputDirectory)
    variant.sources.jniLibs?.addGeneratedSourceDirectory(rustBuilds.getValue(variant.buildType!!), RustExecTask::outputDirectory)
}
tasks.withType<Test>().configureEach {
    dependsOn(buildRustHost)
    systemProperty("jna.library.path", workspace.dir("target/debug").asFile.absolutePath)
}

dependencies {
    api(project(":core:model"))
    implementation("net.java.dev.jna:jna:5.19.1@aar")
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.androidx.core.ktx)
    testImplementation(libs.junit)
    testRuntimeOnly("net.java.dev.jna:jna:5.19.1@jar")
}
