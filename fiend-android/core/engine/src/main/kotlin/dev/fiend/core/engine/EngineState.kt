package dev.fiend.core.engine

import dev.fiend.core.engine.bindings.ProfileConfig
import dev.fiend.core.engine.bindings.TunnelState as NativeState
import dev.fiend.core.engine.bindings.TunnelStats as NativeStats
import dev.fiend.core.engine.bindings.bundledTemplates
import dev.fiend.core.model.ConnectStep
import dev.fiend.core.model.LogLine
import dev.fiend.core.model.Profile
import dev.fiend.core.model.TunnelState
import javax.inject.Inject
import javax.inject.Singleton
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow

object EngineServiceContract {
    const val SERVICE_CLASS = "dev.fiend.android.FiendVpnService"
    const val CONNECT = "dev.fiend.CONNECT"
    const val DISCONNECT = "dev.fiend.DISCONNECT"
    const val PROFILE_ID = "profile_id"
}

fun Profile.engineConfig(): ProfileConfig {
    val json = templateJson ?: bundledTemplates().firstOrNull { it.id == templateId }?.templateJson
        ?: error("The profile template is not available")
    return ProfileConfig(name, authority, secret, json)
}

@Singleton
class EngineState @Inject constructor() {
    private val mutableState = MutableStateFlow<TunnelState>(TunnelState.Disconnected)
    val state = mutableState.asStateFlow()
    private val mutableLogs = MutableSharedFlow<LogLine>(extraBufferCapacity = 256)
    val logs = mutableLogs.asSharedFlow()
    private var generation = 0L
    private var profileId = ""
    private var connectedAt = 0L
    private var statsReader: (() -> NativeStats)? = null

    @Synchronized
    fun begin(id: String): Long {
        generation += 1
        profileId = id
        connectedAt = 0
        statsReader = null
        mutableState.value = TunnelState.Connecting(id, ConnectStep.Dialing)
        return generation
    }

    @Synchronized
    fun update(session: Long, state: NativeState) {
        if (session != generation) return
        mutableState.value = when (state) {
            NativeState.Disconnected -> TunnelState.Disconnected
            NativeState.Connecting -> TunnelState.Connecting(profileId, ConnectStep.Handshake)
            NativeState.Connected -> {
                if (connectedAt == 0L) connectedAt = System.currentTimeMillis()
                TunnelState.Connected(profileId, connectedAt)
            }
            is NativeState.Failed -> TunnelState.Failed(profileId, state.reason)
        }
    }

    @Synchronized
    fun isCurrent(session: Long): Boolean = session == generation

    @Synchronized
    fun attach(session: Long, reader: () -> NativeStats) {
        if (session == generation) statsReader = reader
    }

    @Synchronized
    fun snapshot(): NativeStats? = runCatching { statsReader?.invoke() }.getOrNull()

    @Synchronized
    fun fail(id: String, reason: String) {
        generation += 1
        statsReader = null
        mutableState.value = TunnelState.Failed(id, reason)
    }

    @Synchronized
    fun disconnecting() {
        mutableState.value = TunnelState.Disconnecting
    }

    @Synchronized
    fun disconnected() {
        generation += 1
        statsReader = null
        mutableState.value = TunnelState.Disconnected
    }

    fun log(line: LogLine) { mutableLogs.tryEmit(line) }
}
