package dev.fiend.feature.home

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import dev.fiend.core.data.ProfileRepository
import dev.fiend.core.data.TunnelController
import dev.fiend.core.model.FingerprintTemplate
import dev.fiend.core.model.Profile
import dev.fiend.core.model.TunnelState
import dev.fiend.core.model.TunnelStats
import javax.inject.Inject
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch

data class HomeUiState(
    val tunnel: TunnelState = TunnelState.Disconnected,
    val stats: TunnelStats = TunnelStats.Empty,
    val activeProfile: Profile? = null,
    val activeTemplate: FingerprintTemplate? = null,
    val profiles: List<Profile> = emptyList(),
)

@HiltViewModel
class HomeViewModel @Inject constructor(
    private val profiles: ProfileRepository,
    private val tunnel: TunnelController,
) : ViewModel() {

    val uiState: StateFlow<HomeUiState> = combine(
        tunnel.state,
        tunnel.stats,
        profiles.profiles,
        profiles.activeProfileId,
    ) { state, stats, list, activeId ->
        val active = list.firstOrNull { it.id == activeId } ?: list.firstOrNull()
        HomeUiState(
            tunnel = state,
            stats = stats,
            activeProfile = active,
            activeTemplate = profiles.templates.firstOrNull { it.id == active?.templateId },
            profiles = list,
        )
    }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), HomeUiState())

    fun toggle() {
        when (tunnel.state.value) {
            TunnelState.Disconnected, is TunnelState.Failed -> uiState.value.activeProfile?.let { tunnel.connect(it.id) }
            is TunnelState.Connecting, is TunnelState.Connected -> tunnel.disconnect()
            TunnelState.Disconnecting -> Unit
        }
    }

    fun select(profileId: String) {
        viewModelScope.launch {
            profiles.setActive(profileId)
            when (tunnel.state.value) {
                is TunnelState.Connected, is TunnelState.Connecting -> tunnel.connect(profileId)
                else -> Unit
            }
        }
    }
}
