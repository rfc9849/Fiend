package dev.fiend.feature.profiles.edit

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.assisted.Assisted
import dagger.assisted.AssistedFactory
import dagger.assisted.AssistedInject
import dagger.hilt.android.lifecycle.HiltViewModel
import dev.fiend.core.data.ProfileRepository
import dev.fiend.core.model.FingerprintTemplate
import dev.fiend.core.model.Profile
import dev.fiend.core.model.Tuning
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

enum class Field { Name, Host, Port, Secret, Handshake, Connections, UdpIdle, UdpKeepalive }

enum class FieldError { Required, Port, Secret, Number, Keepalive }

data class ProfileForm(
    val name: String = "",
    val host: String = "",
    val port: String = "443",
    val secret: String = "",
    val templateId: String = "",
    val handshake: String = Tuning().handshakeTimeoutMs.toString(),
    val connections: String = Tuning().maxConnections.toString(),
    val udpIdle: String = Tuning().udpIdleMs.toString(),
    val udpKeepalive: String = Tuning().udpKeepaliveMs.toString(),
)

data class ProfileEditUiState(
    val isNew: Boolean = true,
    val loaded: Boolean = false,
    val form: ProfileForm = ProfileForm(),
    val initial: ProfileForm = ProfileForm(),
    val errors: Map<Field, FieldError> = emptyMap(),
    val templates: List<FingerprintTemplate> = emptyList(),
    val saving: Boolean = false,
) {
    val dirty: Boolean get() = form != initial
}

@HiltViewModel(assistedFactory = ProfileEditViewModel.Factory::class)
class ProfileEditViewModel @AssistedInject constructor(
    @Assisted private val profileId: String?,
    private val repository: ProfileRepository,
) : ViewModel() {

    @AssistedFactory
    interface Factory {
        fun create(profileId: String?): ProfileEditViewModel
    }

    private val state = MutableStateFlow(ProfileEditUiState(isNew = profileId == null, templates = repository.templates))
    val uiState: StateFlow<ProfileEditUiState> = state.asStateFlow()

    private val savedChannel = Channel<Unit>(Channel.CONFLATED)
    val saved = savedChannel.receiveAsFlow()

    private var original: Profile? = null

    init {
        viewModelScope.launch {
            val profile = profileId?.let { repository.profile(it).first() }
            original = profile
            val form = profile?.toForm() ?: ProfileForm(templateId = repository.templates.first().id)
            state.update { it.copy(loaded = true, form = form, initial = form) }
        }
    }

    fun update(transform: (ProfileForm) -> ProfileForm) {
        state.update { current ->
            val form = transform(current.form)
            current.copy(form = form, errors = if (current.errors.isEmpty()) emptyMap() else validate(form))
        }
    }

    fun save() {
        val current = state.value
        val errors = validate(current.form)
        if (errors.isNotEmpty()) {
            state.update { it.copy(errors = errors) }
            return
        }
        state.update { it.copy(saving = true) }
        viewModelScope.launch {
            val f = current.form
            val base = original ?: Profile(id = "", name = "", host = "", port = 443, secret = "", templateId = "")
            repository.upsert(
                base.copy(
                    name = f.name.trim(),
                    host = f.host.trim(),
                    port = f.port.toInt(),
                    secret = f.secret.trim().lowercase(),
                    templateId = f.templateId,
                    tuning = Tuning(
                        handshakeTimeoutMs = f.handshake.toInt(),
                        maxConnections = f.connections.toInt(),
                        udpIdleMs = f.udpIdle.toInt(),
                        udpKeepaliveMs = f.udpKeepalive.toInt(),
                        maxUdpFlows = base.tuning.maxUdpFlows,
                    ),
                ),
            )
            savedChannel.send(Unit)
        }
    }

    private fun validate(f: ProfileForm): Map<Field, FieldError> = buildMap {
        if (f.name.isBlank()) put(Field.Name, FieldError.Required)
        if (f.host.isBlank()) put(Field.Host, FieldError.Required)
        if (f.port.toIntOrNull()?.takeIf { it in 1..65535 } == null) put(Field.Port, FieldError.Port)
        if (!HEX_SECRET.matches(f.secret.trim())) put(Field.Secret, FieldError.Secret)
        listOf(
            Field.Handshake to f.handshake,
            Field.Connections to f.connections,
            Field.UdpIdle to f.udpIdle,
            Field.UdpKeepalive to f.udpKeepalive,
        ).forEach { (field, value) ->
            if ((value.toIntOrNull() ?: 0) <= 0) put(field, FieldError.Number)
        }
        val idle = f.udpIdle.toIntOrNull()
        val keepalive = f.udpKeepalive.toIntOrNull()
        if (idle != null && keepalive != null && keepalive >= idle && !containsKey(Field.UdpKeepalive)) {
            put(Field.UdpKeepalive, FieldError.Keepalive)
        }
    }

    private fun Profile.toForm() = ProfileForm(
        name = name,
        host = host,
        port = port.toString(),
        secret = secret,
        templateId = templateId,
        handshake = tuning.handshakeTimeoutMs.toString(),
        connections = tuning.maxConnections.toString(),
        udpIdle = tuning.udpIdleMs.toString(),
        udpKeepalive = tuning.udpKeepaliveMs.toString(),
    )

    private companion object {
        val HEX_SECRET = Regex("^[0-9a-fA-F]{64}$")
    }
}
