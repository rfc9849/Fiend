package dev.fiend.core.model

sealed interface TunnelState {
    data object Disconnected : TunnelState

    data class Connecting(val profileId: String, val step: ConnectStep) : TunnelState

    data class Connected(val profileId: String, val sinceEpochMs: Long) : TunnelState

    data object Disconnecting : TunnelState

    data class Failed(val profileId: String, val reason: String) : TunnelState
}

enum class ConnectStep { Dialing, Handshake, Routing }

data class TunnelStats(
    val downloadBps: Long = 0,
    val uploadBps: Long = 0,
    val receivedBytes: Long = 0,
    val sentBytes: Long = 0,
    val latencyMs: Int? = null,
    val activeFlows: Int = 0,
) {
    companion object {
        val Empty = TunnelStats()
    }
}
