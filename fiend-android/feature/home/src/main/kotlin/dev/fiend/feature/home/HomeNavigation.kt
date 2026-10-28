package dev.fiend.feature.home

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.navigation3.runtime.EntryProviderScope
import androidx.navigation3.runtime.NavKey
import kotlinx.serialization.Serializable

@Serializable
data object HomeKey : NavKey

fun EntryProviderScope<NavKey>.homeEntry(
    onManageProfiles: () -> Unit,
    onOpenLogs: () -> Unit,
) {
    entry<HomeKey> {
        HomeRoute(onManageProfiles = onManageProfiles, onOpenLogs = onOpenLogs)
    }
}

@Composable
private fun HomeRoute(
    onManageProfiles: () -> Unit,
    onOpenLogs: () -> Unit,
    viewModel: HomeViewModel = hiltViewModel(),
) {
    val state by viewModel.uiState.collectAsStateWithLifecycle()
    HomeScreen(
        state = state,
        onToggle = viewModel::toggle,
        onSelectProfile = viewModel::select,
        onManageProfiles = onManageProfiles,
        onOpenLogs = onOpenLogs,
    )
}
