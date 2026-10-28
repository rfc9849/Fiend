package dev.fiend.android

import android.app.PendingIntent
import android.content.Intent
import android.net.VpnService
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import dagger.hilt.android.AndroidEntryPoint
import dev.fiend.core.data.ProfileRepository
import dev.fiend.core.data.TunnelController
import dev.fiend.core.model.TunnelState
import javax.inject.Inject
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

    override fun onStartListening() {
        super.onStartListening()
        listening = scope.launch {
            tunnel.state.collect { state ->
                qsTile?.apply {
                    this.state = if (state is TunnelState.Connected) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
                    updateTile()
                }
            }
        }
    }

    override fun onStopListening() { listening?.cancel(); super.onStopListening() }

    @android.annotation.SuppressLint("StartActivityAndCollapseDeprecated")
    override fun onClick() {
        super.onClick()
        unlockAndRun {
            when (tunnel.state.value) {
                is TunnelState.Connected, is TunnelState.Connecting -> tunnel.disconnect()
                else -> {
                    val id = profiles.activeProfileId.value ?: profiles.profiles.value.firstOrNull()?.id
                    if (id != null) tunnel.connect(id)
                    if (id == null || VpnService.prepare(this) != null || tunnel.permissionRequired.value) {
                        val intent = Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                        if (Build.VERSION.SDK_INT >= 34) startActivityAndCollapse(PendingIntent.getActivity(this, 0, intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE))
                        else {
                            @Suppress("DEPRECATION")
                            startActivityAndCollapse(intent)
                        }
                    }
                }
            }
        }
    }

    override fun onDestroy() { scope.cancel(); super.onDestroy() }
}
