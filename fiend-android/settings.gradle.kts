pluginManagement {
    includeBuild("build-logic")
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "fiend-android"

include(":app")
include(":core:model")
include(":core:data")
include(":core:engine")
include(":core:designsystem")
include(":feature:home")
include(":feature:profiles")
include(":feature:settings")
