package dev.fiend.android.ui

import androidx.annotation.StringRes
import androidx.compose.animation.ContentTransform
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.viewmodel.navigation3.rememberViewModelStoreNavEntryDecorator
import androidx.navigation3.runtime.NavBackStack
import androidx.navigation3.runtime.NavKey
import androidx.navigation3.runtime.entryProvider
import androidx.navigation3.runtime.rememberNavBackStack
import androidx.navigation3.runtime.rememberSaveableStateHolderNavEntryDecorator
import androidx.navigation3.ui.NavDisplay
import androidx.savedstate.serialization.SavedStateConfiguration
import dev.fiend.android.R
import dev.fiend.core.designsystem.component.FiendBrand
import dev.fiend.core.designsystem.component.FiendNavItem
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.core.designsystem.theme.FiendColors
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

private val TopLevel.icon: ImageVector
    @Composable get() = when (this) {
        TopLevel.Home -> FiendIcons.Home
        TopLevel.Profiles -> FiendIcons.Dns
        TopLevel.Settings -> FiendIcons.Settings
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

    BoxWithConstraints(Modifier.fillMaxSize().background(FiendColors.surface)) {
        val wide = maxWidth >= 600.dp
        val expanded = maxWidth >= 840.dp
        if (wide) {
            Row(Modifier.fillMaxSize()) {
                Sidebar(
                    expanded = expanded,
                    currentTop = currentTop,
                    onSelect = { backStack.selectTopLevel(it.key) },
                )
                Box(Modifier.weight(1f).fillMaxHeight().navigationBarsPadding()) {
                    FiendContent(backStack = backStack, versionName = versionName)
                }
            }
        } else {
            Scaffold(
                containerColor = FiendColors.surface,
                contentWindowInsets = WindowInsets(0, 0, 0, 0),
                bottomBar = {
                    if (showNavigation) {
                        BottomBar(
                            currentTop = currentTop,
                            onSelect = { backStack.selectTopLevel(it.key) },
                        )
                    }
                },
            ) { padding ->
                Box(
                    Modifier
                        .fillMaxSize()
                        .padding(padding)
                        .then(if (showNavigation) Modifier else Modifier.navigationBarsPadding()),
                ) {
                    FiendContent(backStack = backStack, versionName = versionName)
                }
            }
        }
    }
}

@Composable
private fun FiendContent(backStack: NavBackStack<NavKey>, versionName: String) {
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
                onBack = { if (backStack.size > 1) backStack.removeAt(backStack.lastIndex) },
                versionName = versionName,
            )
        },
    )
}

@Composable
private fun Sidebar(
    expanded: Boolean,
    currentTop: NavKey,
    onSelect: (TopLevel) -> Unit,
) {
    Column(
        modifier = Modifier
            .width(if (expanded) 208.dp else 72.dp)
            .fillMaxHeight()
            .background(FiendColors.sidebar)
            .windowInsetsPadding(WindowInsets.safeDrawing)
            .padding(horizontal = if (expanded) 10.dp else 8.dp, vertical = 10.dp),
    ) {
        if (expanded) {
            FiendBrand(
                text = stringResource(R.string.brand),
                modifier = Modifier.padding(start = 12.dp, top = 6.dp, bottom = 26.dp),
            )
        } else {
            Icon(
                FiendIcons.Brand,
                contentDescription = null,
                tint = FiendColors.text,
                modifier = Modifier
                    .align(Alignment.CenterHorizontally)
                    .padding(top = 8.dp, bottom = 26.dp)
                    .size(28.dp),
            )
        }
        TopLevel.entries.filter { it != TopLevel.Settings }.forEach { item ->
            FiendNavItem(
                selected = item.key == currentTop,
                icon = item.icon,
                label = stringResource(item.label),
                onClick = { onSelect(item) },
                showLabel = expanded,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(vertical = 2.dp),
            )
        }
        Spacer(Modifier.weight(1f))
        FiendNavItem(
            selected = TopLevel.Settings.key == currentTop,
            icon = TopLevel.Settings.icon,
            label = stringResource(TopLevel.Settings.label),
            onClick = { onSelect(TopLevel.Settings) },
            showLabel = expanded,
            modifier = Modifier
                .fillMaxWidth()
                .padding(vertical = 2.dp),
        )
    }
}

@Composable
private fun BottomBar(currentTop: NavKey, onSelect: (TopLevel) -> Unit) {
    Column(Modifier.fillMaxWidth().background(FiendColors.sidebar)) {
        HorizontalDivider(color = FiendColors.line, thickness = 1.dp)
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .navigationBarsPadding()
                .padding(horizontal = 16.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            TopLevel.entries.forEach { item ->
                BottomItem(
                    selected = item.key == currentTop,
                    icon = item.icon,
                    label = stringResource(item.label),
                    onClick = { onSelect(item) },
                    modifier = Modifier.weight(1f),
                )
            }
        }
    }
}

@Composable
private fun BottomItem(
    selected: Boolean,
    icon: ImageVector,
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val shape = RoundedCornerShape(4.dp)
    Column(
        modifier = modifier
            .height(60.dp)
            .clip(shape)
            .background(if (selected) FiendColors.panel else Color.Transparent)
            .then(if (selected) Modifier.border(BorderStroke(1.dp, FiendColors.line), shape) else Modifier)
            .clickable(role = Role.Tab, onClick = onClick)
            .padding(vertical = 4.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Icon(
            icon,
            contentDescription = null,
            tint = if (selected) FiendColors.text else FiendColors.muted,
            modifier = Modifier.size(22.dp),
        )
        Text(
            text = label,
            style = MaterialTheme.typography.labelSmall,
            color = if (selected) FiendColors.text else FiendColors.muted,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
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
