package dev.fiend.core.data.mock

import dev.fiend.core.data.ApplicationScope
import dev.fiend.core.data.LogRepository
import dev.fiend.core.data.ProfileRepository
import dev.fiend.core.data.TunnelController
import dev.fiend.core.model.ConnectStep
import dev.fiend.core.model.LogLevel
import dev.fiend.core.model.LogLine
import dev.fiend.core.model.TunnelState
import dev.fiend.core.model.TunnelStats
import javax.inject.Inject
import javax.inject.Singleton
import kotlin.random.Random
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

@Singleton
class MockTunnelController @Inject constructor(
    @ApplicationScope private val scope: CoroutineScope,
    private val profiles: ProfileRepository,
    private val logs: LogRepository,
) : TunnelController {
    private val mutableState = MutableStateFlow<TunnelState>(TunnelState.Disconnected)
    private val mutableStats = MutableStateFlow(TunnelStats.Empty)
    private var job: Job? = null

    override val state: StateFlow<TunnelState> = mutableState.asStateFlow()
    override val stats: StateFlow<TunnelStats> = mutableStats.asStateFlow()

    override fun connect(profileId: String) {
        job?.cancel()
        job = scope.launch {
            val profile = profiles.profiles.value.firstOrNull { it.id == profileId } ?: return@launch
            mutableStats.value = TunnelStats.Empty
            mutableState.value = TunnelState.Connecting(profileId, ConnectStep.Dialing)
            log(LogLevel.Info, "fiend_core::client", "dial ${profile.authority}")
            delay(750)
            mutableState.value = TunnelState.Connecting(profileId, ConnectStep.Handshake)
            log(LogLevel.Debug, "fiend_core::transport", "ClientHello from template ${profile.templateId}")
            delay(850)
            mutableState.value = TunnelState.Connecting(profileId, ConnectStep.Routing)
            log(LogLevel.Info, "fiend_core::mux", "link 1/2 ready, h2 settings acked")
            delay(450)
            mutableState.value = TunnelState.Connected(profileId, System.currentTimeMillis())
            log(LogLevel.Info, "fiend_tun", "tun up, mtu 1500, routes 0.0.0.0/0 ::/0")
            simulateTraffic(profile.latencyMs ?: 60)
        }
    }

    override fun disconnect() {
        job?.cancel()
        job = scope.launch {
            mutableState.value = TunnelState.Disconnecting
            log(LogLevel.Info, "fiend_tun", "tun down")
            delay(420)
            mutableState.value = TunnelState.Disconnected
            mutableStats.value = TunnelStats.Empty
            log(LogLevel.Info, "fiend_core::client", "client stopped")
        }
    }

    private suspend fun CoroutineScope.simulateTraffic(baseLatency: Int) {
        var down = 6_000_000.0
        var up = 900_000.0
        var received = 0L
        var sent = 0L
        while (isActive) {
            down = (down * Random.nextDouble(0.7, 1.35)).coerceIn(300_000.0, 48_000_000.0)
            up = (up * Random.nextDouble(0.7, 1.35)).coerceIn(40_000.0, 6_000_000.0)
            received += (down / 8).toLong()
            sent += (up / 8).toLong()
            mutableStats.value = TunnelStats(
                downloadBps = down.toLong(),
                uploadBps = up.toLong(),
                receivedBytes = received,
                sentBytes = sent,
                latencyMs = (baseLatency + Random.nextInt(-4, 7)).coerceAtLeast(3),
                activeFlows = Random.nextInt(6, 42),
            )
            if (Random.nextInt(12) == 0) {
                log(LogLevel.Debug, "fiend_core::mux::pool", "open lane tcp ${randomHost()}:443")
            }
            delay(1_000)
        }
    }

    private fun randomHost() = listOf("www.google.com", "i.ytimg.com", "api.github.com", "cdn.discordapp.com").random()

    private fun log(level: LogLevel, target: String, message: String) {
        logs.append(LogLine(System.currentTimeMillis(), level, target, message))
    }
}
