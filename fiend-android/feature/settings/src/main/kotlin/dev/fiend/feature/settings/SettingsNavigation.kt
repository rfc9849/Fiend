package dev.fiend.feature.settings

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.navigation3.runtime.EntryProviderScope
import androidx.navigation3.runtime.NavKey
import kotlinx.serialization.Serializable

@Serializable
data object SettingsKey : NavKey

@Serializable
data object SplitTunnelKey : NavKey

@Serializable
data object LogsKey : NavKey

fun EntryProviderScope<NavKey>.settingsEntries(
    onOpenSplitTunnel: () -> Unit,
    onOpenLogs: () -> Unit,
    onBack: () -> Unit,
    versionName: String,
) {
    entry<SettingsKey> {
        SettingsRoute(onOpenSplitTunnel = onOpenSplitTunnel, onOpenLogs = onOpenLogs, versionName = versionName)
    }
    entry<SplitTunnelKey> {
        SplitTunnelRoute(onBack = onBack)
    }
    entry<LogsKey> {
        LogsRoute(onBack = onBack)
    }
}

@Composable
private fun SettingsRoute(
    onOpenSplitTunnel: () -> Unit,
    onOpenLogs: () -> Unit,
    versionName: String,
    viewModel: SettingsViewModel = hiltViewModel(),
) {
    val settings by viewModel.settings.collectAsStateWithLifecycle()
    SettingsScreen(
        settings = settings,
        versionName = versionName,
        onUpdate = viewModel::update,
        onTheme = viewModel::setTheme,
        onOpenSplitTunnel = onOpenSplitTunnel,
        onOpenLogs = onOpenLogs,
    )
}

@Composable
private fun SplitTunnelRoute(onBack: () -> Unit, viewModel: SplitTunnelViewModel = hiltViewModel()) {
    val state by viewModel.uiState.collectAsStateWithLifecycle()
    SplitTunnelScreen(
        state = state,
        onBack = onBack,
        onMode = viewModel::setMode,
        onToggle = viewModel::toggle,
        onQueryChange = viewModel::onQueryChange,
    )
}

@Composable
private fun LogsRoute(onBack: () -> Unit, viewModel: LogsViewModel = hiltViewModel()) {
    val lines by viewModel.lines.collectAsStateWithLifecycle()
    LogsScreen(lines = lines, onBack = onBack, onClear = viewModel::clear)
}
