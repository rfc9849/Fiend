package dev.fiend.core.data

import android.Manifest
import android.os.Build
import android.content.pm.PackageManager
import dev.fiend.core.engine.needsLocalNetworkAccess
import android.content.Context
import android.content.Intent
import android.net.VpnService
import androidx.core.content.ContextCompat
import dagger.hilt.android.qualifiers.ApplicationContext
import dev.fiend.core.engine.EngineServiceContract
import dev.fiend.core.engine.EngineState
import dev.fiend.core.model.TunnelState
import dev.fiend.core.model.TunnelStats
import java.util.concurrent.atomic.AtomicReference
import javax.inject.Inject
import javax.inject.Singleton
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch

@Singleton
class RealTunnelController @Inject constructor(
    @ApplicationContext private val context: Context,
    private val engine: EngineState,
    private val profiles: ProfileRepository,
    private val settings: SettingsRepository,
    logs: LogRepository,
    @ApplicationScope scope: CoroutineScope,
) : TunnelController {
    override val state = engine.state
    private val pending = AtomicReference<String?>(null)
    private val mutablePermissionRequired = MutableStateFlow(false)
    override val permissionRequired = mutablePermissionRequired.asStateFlow()

    override val stats = flow {
        var previous = engine.snapshot()
        var previousTime = System.nanoTime()
        while (true) {
            delay(1_000)
            val current = engine.snapshot()
            val now = System.nanoTime()
            val seconds = (now - previousTime).coerceAtLeast(1).toDouble() / 1_000_000_000
            emit(if (current == null) TunnelStats.Empty else TunnelStats(
                downloadBps = ((current.receivedBytes.toLong() - (previous?.receivedBytes?.toLong() ?: current.receivedBytes.toLong())).coerceAtLeast(0) / seconds).toLong(),
                uploadBps = ((current.sentBytes.toLong() - (previous?.sentBytes?.toLong() ?: current.sentBytes.toLong())).coerceAtLeast(0) / seconds).toLong(),
                receivedBytes = current.receivedBytes.toLong(),
                sentBytes = current.sentBytes.toLong(),
                activeFlows = current.activeFlows.toInt(),
                latencyMs = current.latencyMs?.toInt(),
            ))
            previous = current
            previousTime = now
        }
    }.stateIn(scope, SharingStarted.WhileSubscribed(5_000), TunnelStats.Empty)

    init { scope.launch { engine.logs.collect(logs::append) } }

    override fun connect(profileId: String) {
        runCatching {
            val vpnConsent = !settings.settings.value.rootMode && VpnService.prepare(context) != null
            if (vpnConsent || needsLocalPermission(profileId)) {
                pending.set(profileId)
                mutablePermissionRequired.value = true
            } else {
                engine.begin(profileId)
                ContextCompat.startForegroundService(context, intent(EngineServiceContract.CONNECT).putExtra(EngineServiceContract.PROFILE_ID, profileId))
            }
        }.onFailure { engine.fail(profileId, it.message ?: "Cannot start the VPN service") }
    }

    override fun localNetworkPermissionNeeded(): Boolean = pending.get()?.let(::needsLocalPermission) ?: false

    private fun needsLocalPermission(profileId: String): Boolean {
        if (Build.VERSION.SDK_INT < 37 || ContextCompat.checkSelfPermission(context, Manifest.permission.ACCESS_LOCAL_NETWORK) == PackageManager.PERMISSION_GRANTED) return false
        val host = profiles.profiles.value.firstOrNull { it.id == profileId }?.host.orEmpty()
        return needsLocalNetworkAccess(host, settings.settings.value.bypassLan)
    }

    override fun permissionResult(granted: Boolean) {
        val id = pending.getAndSet(null)
        mutablePermissionRequired.value = false
        if (id == null) return
        if (granted) connect(id) else engine.fail(id, "Connection permission was not granted")
    }

    override fun disconnect() {
        pending.set(null)
        mutablePermissionRequired.value = false
        if (state.value == TunnelState.Disconnected) return
        engine.disconnecting()
        runCatching { ContextCompat.startForegroundService(context, intent(EngineServiceContract.DISCONNECT)) }
            .onFailure { engine.disconnected() }
    }

    private fun intent(action: String) = Intent(action).setClassName(context, EngineServiceContract.SERVICE_CLASS)
}
