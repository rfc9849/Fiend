package dev.fiend.feature.settings

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import dev.fiend.core.data.AppsRepository
import dev.fiend.core.data.LogRepository
import dev.fiend.core.data.SettingsRepository
import dev.fiend.core.model.AppSettings
import dev.fiend.core.model.InstalledApp
import dev.fiend.core.model.LogLine
import dev.fiend.core.model.SplitTunnelMode
import dev.fiend.core.model.ThemeMode
import javax.inject.Inject
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch

@HiltViewModel
class SettingsViewModel @Inject constructor(
    private val repository: SettingsRepository,
) : ViewModel() {
    val settings: StateFlow<AppSettings> = repository.settings

    fun update(transform: (AppSettings) -> AppSettings) {
        viewModelScope.launch { repository.update(transform) }
    }

    fun setTheme(mode: ThemeMode) = update { it.copy(themeMode = mode) }
}

data class SplitTunnelUiState(
    val mode: SplitTunnelMode = SplitTunnelMode.AllApps,
    val selected: Set<String> = emptySet(),
    val apps: List<InstalledApp> = emptyList(),
    val query: String = "",
)

@HiltViewModel
class SplitTunnelViewModel @Inject constructor(
    private val settings: SettingsRepository,
    apps: AppsRepository,
) : ViewModel() {
    private val query = MutableStateFlow("")

    val uiState: StateFlow<SplitTunnelUiState> = combine(settings.settings, apps.apps, query) { s, list, q ->
        val needle = q.trim()
        SplitTunnelUiState(
            mode = s.splitTunnelMode,
            selected = s.splitTunnelApps,
            apps = list.filter { needle.isEmpty() || it.label.contains(needle, true) || it.packageName.contains(needle, true) },
            query = q,
        )
    }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), SplitTunnelUiState())

    fun setMode(mode: SplitTunnelMode) {
        viewModelScope.launch { settings.update { it.copy(splitTunnelMode = mode) } }
    }

    fun toggle(packageName: String) {
        viewModelScope.launch {
            settings.update {
                val next = if (packageName in it.splitTunnelApps) it.splitTunnelApps - packageName else it.splitTunnelApps + packageName
                it.copy(splitTunnelApps = next)
            }
        }
    }

    fun onQueryChange(value: String) {
        query.value = value
    }
}

@HiltViewModel
class LogsViewModel @Inject constructor(
    private val repository: LogRepository,
) : ViewModel() {
    val lines: StateFlow<List<LogLine>> = repository.lines

    fun clear() = repository.clear()
}
