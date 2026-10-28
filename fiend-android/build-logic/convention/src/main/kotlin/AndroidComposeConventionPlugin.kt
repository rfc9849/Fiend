import com.android.build.api.dsl.ApplicationExtension
import com.android.build.api.dsl.LibraryExtension
import dev.fiend.buildlogic.libs
import org.gradle.api.Plugin
import org.gradle.api.Project
import org.gradle.kotlin.dsl.configure
import org.gradle.kotlin.dsl.dependencies
import org.jetbrains.kotlin.compose.compiler.gradle.ComposeCompilerGradlePluginExtension
import org.jetbrains.kotlin.gradle.dsl.KotlinAndroidProjectExtension

class AndroidComposeConventionPlugin : Plugin<Project> {
    override fun apply(target: Project) {
        with(target) {
            pluginManager.apply("org.jetbrains.kotlin.plugin.compose")
            pluginManager.withPlugin("com.android.application") {
                extensions.configure<ApplicationExtension> { buildFeatures.compose = true }
            }
            pluginManager.withPlugin("com.android.library") {
                extensions.configure<LibraryExtension> { buildFeatures.compose = true }
            }
            extensions.configure<ComposeCompilerGradlePluginExtension> {
                includeSourceInformation.set(true)
            }
            extensions.configure<KotlinAndroidProjectExtension> {
                compilerOptions {
                    optIn.addAll(
                        "androidx.compose.material3.ExperimentalMaterial3Api",
                        "androidx.compose.material3.ExperimentalMaterial3ExpressiveApi",
                        "androidx.compose.foundation.layout.ExperimentalLayoutApi",
                        "androidx.compose.animation.ExperimentalSharedTransitionApi",
                    )
                }
            }
            dependencies {
                val bom = platform(libs.findLibrary("androidx-compose-bom").get())
                add("implementation", bom)
                add("implementation", libs.findLibrary("androidx-compose-ui-tooling-preview").get())
                add("debugImplementation", libs.findLibrary("androidx-compose-ui-tooling").get())
            }
        }
    }
}
