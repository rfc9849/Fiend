package dev.fiend.feature.profiles.list

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import dev.fiend.core.data.ProfileRepository
import dev.fiend.core.data.TunnelController
import dev.fiend.core.model.Profile
import dev.fiend.core.model.TunnelState
import javax.inject.Inject
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch

data class ProfilesUiState(
    val loaded: Boolean = false,
    val active: Profile? = null,
    val saved: List<Profile> = emptyList(),
    val connectedId: String? = null,
    val query: String = "",
    val results: List<Profile> = emptyList(),
)

sealed interface ProfilesEvent {
    data class Imported(val name: String) : ProfilesEvent

    data object ImportFailed : ProfilesEvent
}

@HiltViewModel
class ProfilesViewModel @Inject constructor(
    private val repository: ProfileRepository,
    tunnel: TunnelController,
) : ViewModel() {
    private val query = MutableStateFlow("")
    private val eventChannel = Channel<ProfilesEvent>(Channel.BUFFERED)
    val events = eventChannel.receiveAsFlow()

    val uiState: StateFlow<ProfilesUiState> = combine(
        repository.profiles,
        repository.activeProfileId,
        tunnel.state,
        query,
    ) { profiles, activeId, state, q ->
        val active = profiles.firstOrNull { it.id == activeId }
        val needle = q.trim()
        ProfilesUiState(
            loaded = true,
            active = active,
            saved = profiles.filter { it.id != activeId }.sortedByDescending { it.lastConnectedAt ?: 0 },
            connectedId = (state as? TunnelState.Connected)?.profileId,
            query = q,
            results = if (needle.isEmpty()) {
                profiles
            } else {
                profiles.filter { it.name.contains(needle, ignoreCase = true) || it.host.contains(needle, ignoreCase = true) }
            },
        )
    }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), ProfilesUiState())

    fun onQueryChange(value: String) {
        query.value = value
    }

    fun import(link: String) {
        viewModelScope.launch {
            repository.import(link)
                .onSuccess { eventChannel.send(ProfilesEvent.Imported(it.name)) }
                .onFailure { eventChannel.send(ProfilesEvent.ImportFailed) }
        }
    }
}
