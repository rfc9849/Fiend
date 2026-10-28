package dev.fiend.feature.profiles.detail

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.assisted.Assisted
import dagger.assisted.AssistedFactory
import dagger.assisted.AssistedInject
import dagger.hilt.android.lifecycle.HiltViewModel
import dev.fiend.core.data.ProfileRepository
import dev.fiend.core.data.TunnelController
import dev.fiend.core.model.FingerprintTemplate
import dev.fiend.core.model.Profile
import dev.fiend.core.model.TunnelState
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch

data class ProfileDetailUiState(
    val loaded: Boolean = false,
    val profile: Profile? = null,
    val template: FingerprintTemplate? = null,
    val isActive: Boolean = false,
    val isConnected: Boolean = false,
    val isBusy: Boolean = false,
    val testing: Boolean = false,
)

sealed interface ProfileDetailEvent {
    data class LatencyMeasured(val name: String, val ms: Int) : ProfileDetailEvent

    data class Duplicated(val id: String) : ProfileDetailEvent

    data object Deleted : ProfileDetailEvent

    data class Failed(val reason: String) : ProfileDetailEvent
}

@HiltViewModel(assistedFactory = ProfileDetailViewModel.Factory::class)
class ProfileDetailViewModel @AssistedInject constructor(
    @Assisted private val profileId: String,
    private val repository: ProfileRepository,
    private val tunnel: TunnelController,
) : ViewModel() {

    @AssistedFactory
    interface Factory {
        fun create(profileId: String): ProfileDetailViewModel
    }

    private val testing = MutableStateFlow(false)
    private val eventChannel = Channel<ProfileDetailEvent>(Channel.BUFFERED)
    val events = eventChannel.receiveAsFlow()

    val uiState: StateFlow<ProfileDetailUiState> = combine(
        repository.profile(profileId),
        repository.activeProfileId,
        tunnel.state,
        testing,
    ) { profile, activeId, state, isTesting ->
        ProfileDetailUiState(
            loaded = true,
            profile = profile,
            template = repository.templates.firstOrNull { it.id == profile?.templateId },
            isActive = activeId == profileId,
            isConnected = (state as? TunnelState.Connected)?.profileId == profileId,
            isBusy = (state as? TunnelState.Connecting)?.profileId == profileId,
            testing = isTesting,
        )
    }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), ProfileDetailUiState())

    fun toggleConnection() {
        val state = uiState.value
        operation {
            if (state.isConnected || state.isBusy) {
                tunnel.disconnect()
            } else {
                repository.setActive(profileId)
                tunnel.connect(profileId)
            }
        }
    }

    fun test() {
        if (testing.value) return
        operation {
            testing.value = true
            try {
                val ms = repository.measureLatency(profileId)
                eventChannel.send(ProfileDetailEvent.LatencyMeasured(uiState.value.profile?.name.orEmpty(), ms))
            } finally { testing.value = false }
        }
    }

    fun duplicate() {
        operation {
            repository.duplicate(profileId)?.let { eventChannel.send(ProfileDetailEvent.Duplicated(it.id)) }
        }
    }

    fun delete() {
        operation {
            if (uiState.value.isConnected) tunnel.disconnect()
            repository.delete(profileId)
            eventChannel.send(ProfileDetailEvent.Deleted)
        }
    }

    fun shareLink(): String = try { uiState.value.profile?.let(repository::shareLink).orEmpty() }
        catch (error: Exception) { eventChannel.trySend(ProfileDetailEvent.Failed(error.message ?: "Cannot share the profile")); "" }

    private fun operation(block: suspend () -> Unit) {
        viewModelScope.launch {
            try { block() }
            catch (error: Exception) {
                if (error is kotlinx.coroutines.CancellationException) throw error
                eventChannel.send(ProfileDetailEvent.Failed(error.message ?: "Profile action failed"))
            }
        }
    }
}
