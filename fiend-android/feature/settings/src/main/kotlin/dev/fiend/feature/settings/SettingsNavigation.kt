package dev.fiend.feature.settings

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
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
data object RoutingKey : NavKey

@Serializable
data class RuleEditorKey(val block: Boolean) : NavKey

@Serializable
data object LogsKey : NavKey

fun EntryProviderScope<NavKey>.settingsEntries(
    onOpenSplitTunnel: () -> Unit,
    onOpenRouting: () -> Unit,
    onOpenRuleEditor: (Boolean) -> Unit,
    onOpenLogs: () -> Unit,
    onBack: () -> Unit,
    versionName: String,
) {
    entry<SettingsKey> {
        SettingsRoute(onOpenSplitTunnel = onOpenSplitTunnel, onOpenRouting = onOpenRouting, onOpenLogs = onOpenLogs, versionName = versionName)
    }
    entry<SplitTunnelKey> {
        SplitTunnelRoute(onBack = onBack)
    }
    entry<RoutingKey> {
        RoutingRoute(onBack = onBack, onOpenRuleEditor = onOpenRuleEditor)
    }
    entry<RuleEditorKey> { key ->
        RuleEditorEntry(key, onBack = onBack)
    }
    entry<LogsKey> {
        LogsRoute(onBack = onBack)
    }
}

@Composable
private fun SettingsRoute(
    onOpenSplitTunnel: () -> Unit,
    onOpenRouting: () -> Unit,
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
        onOpenRouting = onOpenRouting,
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
private fun RoutingRoute(
    onBack: () -> Unit,
    onOpenRuleEditor: (Boolean) -> Unit,
    viewModel: RoutingViewModel = hiltViewModel(),
) {
    val state by viewModel.uiState.collectAsStateWithLifecycle()
    RoutingScreen(
        state = state,
        onBack = onBack,
        onOpenDirect = { onOpenRuleEditor(false) },
        onOpenBlock = { onOpenRuleEditor(true) },
        onDirectMode = viewModel::setDirectMode,
        onResolver = viewModel::setResolver,
        onMtu = viewModel::setMtu,
    )
}

@Composable
private fun RuleEditorEntry(key: RuleEditorKey, onBack: () -> Unit) {
    val viewModel = hiltViewModel<RuleEditorViewModel, RuleEditorViewModel.Factory>(
        creationCallback = { it.create(key.block) },
    )
    val state by viewModel.uiState.collectAsStateWithLifecycle()
    LaunchedEffect(viewModel) { viewModel.closeEvents.collect { onBack() } }
    RuleEditorScreen(
        block = key.block,
        state = state,
        onText = viewModel::onText,
        onImport = viewModel::import,
        onImportShown = viewModel::consumeImported,
        onSave = viewModel::save,
        onClose = onBack,
    )
}

@Composable
private fun LogsRoute(onBack: () -> Unit, viewModel: LogsViewModel = hiltViewModel()) {
    val lines by viewModel.lines.collectAsStateWithLifecycle()
    LogsScreen(lines = lines, onBack = onBack, onClear = viewModel::clear)
}
