package dev.fiend.android

import android.app.PendingIntent
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.net.VpnService
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import dagger.hilt.android.AndroidEntryPoint
import dagger.hilt.android.qualifiers.ApplicationContext
import dev.fiend.core.data.ApplicationScope
import dev.fiend.core.data.ProfileRepository
import dev.fiend.core.data.TunnelController
import dev.fiend.core.model.TunnelState
import javax.inject.Inject
import javax.inject.Singleton
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

@AndroidEntryPoint
class FiendTileService : TileService() {
    @Inject lateinit var tunnel: TunnelController
    @Inject lateinit var profiles: ProfileRepository
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var listening: Job? = null

    override fun onTileAdded() {
        super.onTileAdded()
        requestTileListening(this)
    }

    override fun onStartListening() {
        super.onStartListening()
        listening?.cancel()
        listening = scope.launch {
            tunnel.state.collect { state -> render(state) }
        }
    }

    override fun onStopListening() {
        listening?.cancel()
        listening = null
        super.onStopListening()
    }

    @android.annotation.SuppressLint("StartActivityAndCollapseDeprecated")
    override fun onClick() {
        super.onClick()
        unlockAndRun {
            when (tunnel.state.value) {
                is TunnelState.Connected, is TunnelState.Connecting -> tunnel.disconnect()
                TunnelState.Disconnecting -> Unit
                TunnelState.Disconnected, is TunnelState.Failed -> connect()
            }
            render(tunnel.state.value)
        }
    }

    private fun connect() {
        val id = profiles.activeProfileId.value ?: profiles.profiles.value.firstOrNull()?.id
        if (id != null) tunnel.connect(id)
        if (id == null || VpnService.prepare(this) != null || tunnel.permissionRequired.value) openApp()
    }

    private fun render(state: TunnelState) {
        val tile = qsTile ?: return
        val subtitle = getString(
            when (state) {
                TunnelState.Disconnected -> R.string.tile_state_disconnected
                is TunnelState.Connecting -> R.string.tile_state_connecting
                is TunnelState.Connected -> R.string.tile_state_connected
                TunnelState.Disconnecting -> R.string.tile_state_disconnecting
                is TunnelState.Failed -> R.string.tile_state_failed
            },
        )
        tile.label = getString(R.string.app_name)
        if (Build.VERSION.SDK_INT >= 29) tile.subtitle = subtitle
        tile.state = when (state) {
            is TunnelState.Connected, is TunnelState.Connecting -> Tile.STATE_ACTIVE
            TunnelState.Disconnected, TunnelState.Disconnecting, is TunnelState.Failed -> Tile.STATE_INACTIVE
        }
        tile.contentDescription = "${getString(R.string.app_name)}: $subtitle"
        tile.updateTile()
    }

    @android.annotation.SuppressLint("StartActivityAndCollapseDeprecated")
    private fun openApp() {
        val intent = Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        if (Build.VERSION.SDK_INT >= 34) {
            startActivityAndCollapse(
                PendingIntent.getActivity(this, 0, intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE),
            )
        } else {
            @Suppress("DEPRECATION")
            startActivityAndCollapse(intent)
        }
    }

    override fun onDestroy() { scope.cancel(); super.onDestroy() }
}

@Singleton
class TileStateUpdater @Inject constructor(
    @ApplicationContext private val context: Context,
    @ApplicationScope scope: CoroutineScope,
    tunnel: TunnelController,
) {
    init {
        scope.launch {
            tunnel.state.collect { requestTileListening(context) }
        }
    }
}

private fun requestTileListening(context: Context) {
    runCatching {
        TileService.requestListeningState(context, ComponentName(context, FiendTileService::class.java))
    }
}
