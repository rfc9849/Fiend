package dev.fiend.android

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.VpnService
import android.os.Build
import androidx.core.app.NotificationCompat
import dagger.hilt.android.AndroidEntryPoint
import dev.fiend.core.data.ProfileRepository
import dev.fiend.core.data.SettingsRepository
import dev.fiend.core.designsystem.R as DesignR
import dev.fiend.core.engine.EngineServiceContract
import dev.fiend.core.engine.EngineState
import dev.fiend.core.engine.engineConfig
import dev.fiend.core.engine.bindings.LogLine as NativeLog
import dev.fiend.core.engine.bindings.Platform
import dev.fiend.core.engine.bindings.TunOptions
import dev.fiend.core.engine.bindings.Tunnel
import dev.fiend.core.engine.bindings.TunnelState as NativeState
import dev.fiend.core.model.LogLevel
import dev.fiend.core.model.LogLine
import dev.fiend.core.model.SplitTunnelMode
import dev.fiend.core.model.TunnelState
import java.lang.ref.WeakReference
import javax.inject.Inject
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@AndroidEntryPoint
class FiendVpnService : VpnService() {
    @Inject lateinit var profiles: ProfileRepository
    @Inject lateinit var settings: SettingsRepository
    @Inject lateinit var engine: EngineState
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val nativeLock = Any()
    private var tunnel: Tunnel? = null
    private var connecting: Job? = null
    @Volatile private var destroyed = false
    private var currentProfileId: String? = null
    private var profileName = "fiend"
    private var underlying: Network? = null
    private var watchingNetwork = false
    private val networks = linkedSetOf<Network>()
    private val connectivity by lazy { getSystemService(ConnectivityManager::class.java) }
    private val notifications by lazy { getSystemService(NotificationManager::class.java) }
    private val networkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) { scope.launch { networks += network; setNetwork(findUnderlying()) } }
        override fun onLost(network: Network) {
            scope.launch { networks -= network; if (network == underlying) setNetwork(findUnderlying()) }
        }
        override fun onCapabilitiesChanged(network: Network, capabilities: NetworkCapabilities) {
            scope.launch { if (network == underlying) setUnderlyingNetworks(arrayOf(network)) }
        }
    }

    override fun onCreate() {
        super.onCreate()
        notifications.createNotificationChannel(NotificationChannel(CHANNEL, "VPN connection", NotificationManager.IMPORTANCE_LOW).apply { setSound(null, null) })
        watchingNetwork = true
        connectivity.registerNetworkCallback(NetworkRequest.Builder().addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET).addCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN).build(), networkCallback)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        showForeground("Starting")
        if (intent?.action == EngineServiceContract.DISCONNECT) {
            disconnect()
            return START_NOT_STICKY
        }
        connecting?.cancel()
        connecting = scope.launch {
            try {
            profiles.awaitReady()
            settings.awaitReady()
            val list = profiles.profiles.value
            val id = intent?.getStringExtra(EngineServiceContract.PROFILE_ID)
                ?: profiles.activeProfileId.value ?: list.firstOrNull()?.id
            if (id == null) {
                engine.fail("", "Add a profile before you start the VPN")
                disconnect(keepFailure = true)
                return@launch
            }
            connect(id)
            } catch (error: Exception) {
                if (error is kotlinx.coroutines.CancellationException) throw error
                engine.fail("", error.message ?: "Cannot read the VPN settings")
                disconnect(keepFailure = true)
            }
        }
        return START_STICKY
    }

    private suspend fun connect(id: String) {
        closeNative()
        currentProfileId = id
        val session = engine.begin(id)
        try {
            check(prepare(this) == null) { "VPN permission is required" }
            val profile = profiles.profile(id).first() ?: error("The selected profile is not available")
            val appSettings = settings.settings.first()
            val config = profile.engineConfig()
            profileName = profile.name
            underlying = findUnderlying()
            val builder = Builder().setSession(profile.name).setMtu(1500)
                .addAddress("10.42.0.2", 32)
                .addAddress("fd42:6669:656e:6400::2", 128)
                .addRoute("0.0.0.0", 0)
                .addRoute("::", 0)
                .addDnsServer("198.18.0.1")
                .setUnderlyingNetworks(underlying?.let { arrayOf(it) })
            if (Build.VERSION.SDK_INT >= 29) builder.setMetered(false)
            var allowed = 0
            for (packageName in appSettings.splitTunnelApps) {
                try {
                    when (appSettings.splitTunnelMode) {
                        SplitTunnelMode.OnlySelected -> { builder.addAllowedApplication(packageName); allowed += 1 }
                        SplitTunnelMode.ExceptSelected -> builder.addDisallowedApplication(packageName)
                        SplitTunnelMode.AllApps -> Unit
                    }
                } catch (_: PackageManager.NameNotFoundException) { }
            }
            check(appSettings.splitTunnelMode != SplitTunnelMode.OnlySelected || allowed > 0) { "Select at least one installed app" }
            val options = TunOptions(1500u, appSettings.ipv6, appSettings.bypassLan,
                profile.tuning.handshakeTimeoutMs.toULong(), profile.tuning.maxConnections.toUInt(),
                profile.tuning.udpIdleMs.toULong(), profile.tuning.udpKeepaliveMs.toULong(), profile.tuning.maxUdpFlows.toUInt())
            val platform = ServicePlatform(this, session)
            withContext(Dispatchers.IO) {
                if (!engine.isCurrent(session)) return@withContext
                val descriptor = builder.establish() ?: error("Android could not create the VPN interface")
                val fd = descriptor.detachFd()
                descriptor.close()
                val native = Tunnel.start(fd, config, options, platform)
                val retain = synchronized(nativeLock) {
                    if (destroyed || !engine.isCurrent(session)) false else { tunnel = native; true }
                }
                if (retain) {
                    val weak = WeakReference(native)
                    engine.attach(session) { weak.get()?.stats() ?: error("Tunnel is closed") }
                } else { native.stop(); native.close() }
            }
        } catch (error: Exception) {
            if (engine.isCurrent(session)) {
                engine.fail(id, error.message ?: "Cannot start the tunnel")
                showForeground("Connection failed")
            }
        }
    }

    private fun setNetwork(network: Network?) {
        if (network == underlying) return
        underlying = network
        setUnderlyingNetworks(network?.let { arrayOf(it) } ?: emptyArray())
        if (engine.state.value is TunnelState.Failed && network != null) {
            val id = currentProfileId ?: return
            connecting?.cancel()
            connecting = scope.launch { connect(id) }
        } else {
            val native = synchronized(nativeLock) { tunnel }
            runCatching { native?.networkChanged() }
        }
    }

    private fun findUnderlying(): Network? {
        val active = connectivity.activeNetwork
        if (active != null && connectivity.getNetworkCapabilities(active)?.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN) == true) return active
        return networks.firstOrNull { connectivity.getNetworkCapabilities(it)?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true } ?: networks.firstOrNull()
    }

    private fun closeNative() {
        val native = synchronized(nativeLock) { val result = tunnel; tunnel = null; result }
        runCatching { native?.stop() }
        runCatching { native?.close() }
    }

    private fun disconnect(keepFailure: Boolean = false) {
        connecting?.cancel()
        if (!keepFailure) engine.disconnected()
        closeNative()
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    override fun onRevoke() { disconnect(); super.onRevoke() }

    override fun onDestroy() {
        synchronized(nativeLock) { destroyed = true }
        if (engine.state.value !is TunnelState.Failed) engine.disconnected()
        closeNative()
        if (watchingNetwork) runCatching { connectivity.unregisterNetworkCallback(networkCallback) }
        scope.cancel()
        super.onDestroy()
    }

    private fun nativeState(session: Long, state: NativeState) {
        if (destroyed) return
        engine.update(session, state)
        scope.launch {
            if (!engine.isCurrent(session)) return@launch
            if (state == NativeState.Connected) currentProfileId?.let { id -> runCatching { profiles.markConnected(id, System.currentTimeMillis()) } }
            showForeground(when (state) {
                NativeState.Connected -> "Connected"
                NativeState.Connecting -> "Connecting"
                NativeState.Disconnected -> "Disconnected"
                is NativeState.Failed -> "Connection failed"
            })
        }
    }

    private fun showForeground(status: String) {
        val launch = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val disconnect = PendingIntent.getService(this, 1, Intent(this, FiendVpnService::class.java).setAction(EngineServiceContract.DISCONNECT), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val notification: Notification = NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(DesignR.drawable.ic_shield).setContentTitle(profileName).setContentText(status)
            .setContentIntent(launch).setOngoing(true).setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .addAction(DesignR.drawable.ic_power_settings_new, "Disconnect", disconnect).build()
        if (Build.VERSION.SDK_INT >= 34) startForeground(NOTIFICATION, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SYSTEM_EXEMPTED)
        else startForeground(NOTIFICATION, notification)
    }

    private class ServicePlatform(service: FiendVpnService, private val session: Long) : Platform {
        private val reference = WeakReference(service)
        override fun protect(fd: Int): Boolean = reference.get()?.protect(fd) ?: false
        override fun onState(state: NativeState) { reference.get()?.nativeState(session, state) }
        override fun onLog(line: NativeLog) {
            reference.get()?.engine?.log(LogLine(line.timestampMs.toLong(), when (line.level) {
                "ERROR" -> LogLevel.Error
                "WARN" -> LogLevel.Warn
                "DEBUG", "TRACE" -> LogLevel.Debug
                else -> LogLevel.Info
            }, "engine", line.message))
        }
    }

    private companion object { const val CHANNEL = "vpn"; const val NOTIFICATION = 1 }
}
