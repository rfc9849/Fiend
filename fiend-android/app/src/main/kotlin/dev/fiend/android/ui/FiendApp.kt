package dev.fiend.android.ui

import androidx.annotation.StringRes
import androidx.compose.animation.ContentTransform
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.adaptive.currentWindowAdaptiveInfoV2
import androidx.compose.material3.adaptive.navigationsuite.NavigationSuiteItem
import androidx.compose.material3.adaptive.navigationsuite.NavigationSuiteScaffold
import androidx.compose.material3.adaptive.navigationsuite.NavigationSuiteScaffoldDefaults
import androidx.compose.material3.adaptive.navigationsuite.rememberNavigationSuiteScaffoldState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.lifecycle.viewmodel.navigation3.rememberViewModelStoreNavEntryDecorator
import androidx.navigation3.runtime.NavBackStack
import androidx.navigation3.runtime.NavKey
import androidx.navigation3.runtime.entryProvider
import androidx.navigation3.runtime.rememberNavBackStack
import androidx.navigation3.runtime.rememberSaveableStateHolderNavEntryDecorator
import androidx.navigation3.ui.NavDisplay
import androidx.savedstate.serialization.SavedStateConfiguration
import dev.fiend.android.R
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.feature.home.HomeKey
import dev.fiend.feature.home.homeEntry
import dev.fiend.feature.profiles.ProfileDetailKey
import dev.fiend.feature.profiles.ProfileEditKey
import dev.fiend.feature.profiles.ProfilesKey
import dev.fiend.feature.profiles.profilesEntries
import dev.fiend.feature.settings.LogsKey
import dev.fiend.feature.settings.RoutingKey
import dev.fiend.feature.settings.RuleEditorKey
import dev.fiend.feature.settings.SettingsKey
import dev.fiend.feature.settings.SplitTunnelKey
import dev.fiend.feature.settings.settingsEntries
import kotlinx.serialization.modules.SerializersModule
import kotlinx.serialization.modules.polymorphic

private val navConfig = SavedStateConfiguration {
    serializersModule = SerializersModule {
        polymorphic(NavKey::class) {
            subclass(HomeKey::class, HomeKey.serializer())
            subclass(ProfilesKey::class, ProfilesKey.serializer())
            subclass(ProfileDetailKey::class, ProfileDetailKey.serializer())
            subclass(ProfileEditKey::class, ProfileEditKey.serializer())
            subclass(SettingsKey::class, SettingsKey.serializer())
            subclass(SplitTunnelKey::class, SplitTunnelKey.serializer())
            subclass(RoutingKey::class, RoutingKey.serializer())
            subclass(RuleEditorKey::class, RuleEditorKey.serializer())
            subclass(LogsKey::class, LogsKey.serializer())
        }
    }
}

private enum class TopLevel(
    val key: NavKey,
    @param:StringRes val label: Int,
) {
    Home(HomeKey, R.string.nav_home),
    Profiles(ProfilesKey, R.string.nav_profiles),
    Settings(SettingsKey, R.string.nav_settings),
}

@Composable
private fun TopLevel.icon(selected: Boolean): ImageVector = when (this) {
    TopLevel.Home -> if (selected) FiendIcons.HomeFilled else FiendIcons.Home
    TopLevel.Profiles -> if (selected) FiendIcons.DnsFilled else FiendIcons.Dns
    TopLevel.Settings -> if (selected) FiendIcons.SettingsFilled else FiendIcons.Settings
}

private val topLevelKeys = TopLevel.entries.map { it.key }.toSet()

private fun NavBackStack<NavKey>.selectTopLevel(key: NavKey) {
    while (size > 1) removeAt(lastIndex)
    if (key != HomeKey) add(key)
}

private fun NavBackStack<NavKey>.push(key: NavKey) {
    if (lastOrNull() != key) add(key)
}

@Composable
fun FiendApp(versionName: String) {
    val backStack = rememberNavBackStack(navConfig, HomeKey)
    val current = backStack.last()
    val currentTop = backStack.lastOrNull { it in topLevelKeys } ?: HomeKey
    val showNavigation = current in topLevelKeys
    val suiteType = NavigationSuiteScaffoldDefaults.navigationSuiteType(currentWindowAdaptiveInfoV2())
    val suiteState = rememberNavigationSuiteScaffoldState()
    LaunchedEffect(showNavigation) {
        if (showNavigation) suiteState.show() else suiteState.hide()
    }

    NavigationSuiteScaffold(
        navigationSuiteType = suiteType,
        state = suiteState,
        containerColor = MaterialTheme.colorScheme.surface,
        navigationItems = {
            TopLevel.entries.forEach { destination ->
                val selected = destination.key == currentTop
                NavigationSuiteItem(
                    selected = selected,
                    onClick = { backStack.selectTopLevel(destination.key) },
                    icon = { Icon(destination.icon(selected), contentDescription = null) },
                    label = { Text(stringResource(destination.label)) },
                    navigationSuiteType = suiteType,
                )
            }
        },
    ) {
        NavDisplay(
            backStack = backStack,
            onBack = { if (backStack.size > 1) backStack.removeAt(backStack.lastIndex) },
            entryDecorators = listOf(
                rememberSaveableStateHolderNavEntryDecorator(),
                rememberViewModelStoreNavEntryDecorator(),
            ),
            transitionSpec = { forward() },
            popTransitionSpec = { backward() },
            predictivePopTransitionSpec = { backward() },
            entryProvider = entryProvider {
                homeEntry(
                    onManageProfiles = { backStack.selectTopLevel(ProfilesKey) },
                    onOpenLogs = { backStack.push(LogsKey) },
                )
                profilesEntries(
                    onOpenProfile = { backStack.push(ProfileDetailKey(it)) },
                    onEditProfile = { backStack.push(ProfileEditKey(it)) },
                    onBack = { if (backStack.size > 1) backStack.removeAt(backStack.lastIndex) },
                )
                settingsEntries(
                    onOpenSplitTunnel = { backStack.push(SplitTunnelKey) },
                    onOpenRouting = { backStack.push(RoutingKey) },
                    onOpenRuleEditor = { backStack.push(RuleEditorKey(it)) },
                    onOpenLogs = { backStack.push(LogsKey) },
                    onBack = { if (backStack.size > 1) backStack.removeAt(backStack.lastIndex) },
                    versionName = versionName,
                )
            },
        )
    }
}

private const val ENTER_MS = 350
private const val EXIT_MS = 200

private fun forward(): ContentTransform =
    (slideInHorizontally(tween(ENTER_MS)) { it / 6 } + fadeIn(tween(ENTER_MS, delayMillis = 60)))
        .togetherWith(slideOutHorizontally(tween(ENTER_MS)) { -it / 10 } + fadeOut(tween(EXIT_MS)))

private fun backward(): ContentTransform =
    (slideInHorizontally(tween(ENTER_MS)) { -it / 10 } + fadeIn(tween(ENTER_MS, delayMillis = 60)))
        .togetherWith(slideOutHorizontally(tween(ENTER_MS)) { it / 6 } + fadeOut(tween(EXIT_MS)))
