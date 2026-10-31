package dev.fiend.feature.home

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.fiend.core.designsystem.component.FiendIconButton
import dev.fiend.core.designsystem.component.FiendTopBar
import dev.fiend.core.designsystem.component.ShapeAvatar
import dev.fiend.core.designsystem.component.StatTile
import dev.fiend.core.designsystem.component.avatarLabel
import dev.fiend.core.designsystem.component.formatBitrate
import dev.fiend.core.designsystem.component.formatBytes
import dev.fiend.core.designsystem.component.formatElapsed
import dev.fiend.core.designsystem.component.latencyColor
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.core.designsystem.theme.FiendColors
import dev.fiend.core.model.ConnectStep
import dev.fiend.core.model.Profile
import dev.fiend.core.model.TunnelState
import dev.fiend.core.model.TunnelStats
import kotlinx.coroutines.delay

@Composable
internal fun HomeScreen(
    state: HomeUiState,
    onToggle: () -> Unit,
    onSelectProfile: (String) -> Unit,
    onManageProfiles: () -> Unit,
    onOpenLogs: () -> Unit,
) {
    var pickerOpen by rememberSaveable { mutableStateOf(false) }
    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(FiendColors.surface),
    ) {
        FiendTopBar(
            brand = stringResource(R.string.home_brand),
            actions = {
                FiendIconButton(
                    icon = FiendIcons.Article,
                    label = stringResource(R.string.home_logs),
                    onClick = onOpenLogs,
                )
            },
        )
        Column(
            modifier = Modifier
                .weight(1f)
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Column(
                modifier = Modifier
                    .widthIn(max = 680.dp)
                    .padding(top = 8.dp, bottom = 24.dp),
            ) {
                HeroPanel(state = state, onToggle = onToggle)
                Spacer(Modifier.height(16.dp))
                ServerCard(profile = state.activeProfile, onClick = { pickerOpen = true })
                Spacer(Modifier.height(16.dp))
                StatsGrid(state.stats, connected = state.tunnel is TunnelState.Connected)
            }
        }
    }
    if (pickerOpen) {
        ProfilePickerSheet(
            profiles = state.profiles,
            selectedId = state.activeProfile?.id,
            onSelect = {
                onSelectProfile(it)
                pickerOpen = false
            },
            onManage = {
                pickerOpen = false
                onManageProfiles()
            },
            onDismiss = { pickerOpen = false },
        )
    }
}

private val TunnelState.phase: ConnectPhase
    get() = when (this) {
        TunnelState.Disconnected -> ConnectPhase.Idle
        is TunnelState.Connecting, TunnelState.Disconnecting -> ConnectPhase.Busy
        is TunnelState.Connected -> ConnectPhase.Active
        is TunnelState.Failed -> ConnectPhase.Error
    }

@Composable
private fun HeroPanel(state: HomeUiState, onToggle: () -> Unit) {
    val phase = state.tunnel.phase
    val failed = phase == ConnectPhase.Error
    Surface(
        color = FiendColors.hero,
        contentColor = FiendColors.text,
        shape = RoundedCornerShape(7.dp),
        border = BorderStroke(1.dp, if (failed) FiendColors.danger else FiendColors.line),
        modifier = Modifier.fillMaxWidth(),
    ) {
        BoxWithConstraints(
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(min = 300.dp),
        ) {
            val horizontal = maxWidth >= 430.dp
            val horizontalPadding = if (horizontal) 36.dp else 24.dp
            val description = stringResource(
                if (phase == ConnectPhase.Idle || phase == ConnectPhase.Error) R.string.home_action_connect else R.string.home_action_disconnect,
            )
            val button = @Composable {
                ConnectButton(
                    phase = phase,
                    onClick = onToggle,
                    contentDescription = description,
                    stateDescription = statusTitle(state.tunnel),
                    size = if (horizontal) 176.dp else 168.dp,
                )
            }
            if (horizontal) {
                Row(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(horizontal = horizontalPadding, vertical = 24.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    button()
                    Spacer(Modifier.width(38.dp))
                    StateText(state, phase, Modifier.weight(1f))
                }
            } else {
                Column(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(horizontal = horizontalPadding, vertical = 24.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.Center,
                ) {
                    button()
                    Spacer(Modifier.height(16.dp))
                    StateText(state, phase, Modifier.fillMaxWidth(), centered = true)
                }
            }
        }
    }
}

@Composable
private fun StateText(state: HomeUiState, phase: ConnectPhase, modifier: Modifier, centered: Boolean = false) {
    val alignment = if (centered) TextAlign.Center else TextAlign.Start
    Column(
        modifier = modifier,
        horizontalAlignment = if (centered) Alignment.CenterHorizontally else Alignment.Start,
    ) {
        AnimatedContent(
            targetState = statusTitle(state.tunnel),
            transitionSpec = { fadeIn().togetherWith(fadeOut()) },
            label = "status",
        ) { title ->
            Text(
                text = title,
                style = MaterialTheme.typography.displayLarge.copy(fontWeight = FontWeight.Medium),
                color = if (phase == ConnectPhase.Error) FiendColors.danger else FiendColors.text,
                textAlign = alignment,
            )
        }
        Spacer(Modifier.height(12.dp))
        StatusDetail(state, alignment)
    }
}

@Composable
private fun statusTitle(tunnel: TunnelState): String = stringResource(
    when (tunnel) {
        TunnelState.Disconnected -> R.string.home_status_off
        is TunnelState.Connecting -> R.string.home_status_connecting
        is TunnelState.Connected -> R.string.home_status_on
        TunnelState.Disconnecting -> R.string.home_status_stopping
        is TunnelState.Failed -> R.string.home_status_failed
    },
)

@Composable
private fun StatusDetail(state: HomeUiState, alignment: TextAlign) {
    val tunnel = state.tunnel
    val style = MaterialTheme.typography.bodySmall.copy(
        fontFamily = FontFamily.Monospace,
        fontSize = 13.sp,
        lineHeight = 20.sp,
    )
    when (tunnel) {
        is TunnelState.Connected -> {
            val elapsed by produceState(0L, tunnel.sinceEpochMs) {
                while (true) {
                    value = System.currentTimeMillis() - tunnel.sinceEpochMs
                    delay(1_000)
                }
            }
            Text(
                text = formatElapsed(elapsed),
                style = style,
                color = FiendColors.muted,
                textAlign = alignment,
            )
        }
        is TunnelState.Connecting -> Text(
            text = stringResource(
                when (tunnel.step) {
                    ConnectStep.Dialing -> R.string.home_step_dialing
                    ConnectStep.Handshake -> R.string.home_step_handshake
                    ConnectStep.Routing -> R.string.home_step_routing
                },
            ),
            style = style,
            color = FiendColors.muted,
            textAlign = alignment,
        )
        is TunnelState.Failed -> Text(
            text = tunnel.reason,
            style = style,
            color = FiendColors.danger,
            textAlign = alignment,
        )
        else -> Text(
            text = state.activeProfile?.let { stringResource(R.string.home_hint_off, it.name) }
                ?: stringResource(R.string.home_hint_no_profile),
            style = style,
            color = FiendColors.muted,
            textAlign = alignment,
        )
    }
}

@Composable
private fun ServerCard(profile: Profile?, onClick: () -> Unit) {
    val shape = RoundedCornerShape(6.dp)
    Surface(
        onClick = onClick,
        color = FiendColors.panel,
        contentColor = FiendColors.text,
        shape = shape,
        border = BorderStroke(1.dp, FiendColors.line),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(min = 76.dp)
                .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (profile != null) {
                ShapeAvatar(label = avatarLabel(profile.name, profile.countryCode), seed = profile.id, size = 48.dp)
            } else {
                Box(
                    modifier = Modifier
                        .size(48.dp)
                        .background(FiendColors.soft, RoundedCornerShape(4.dp)),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(FiendIcons.Dns, contentDescription = null, tint = FiendColors.muted, modifier = Modifier.size(20.dp))
                }
            }
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text(
                    text = profile?.name ?: stringResource(R.string.home_no_server),
                    style = MaterialTheme.typography.titleMedium,
                    color = FiendColors.text,
                    maxLines = 1,
                )
                Text(
                    text = profile?.authority ?: stringResource(R.string.home_server),
                    style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace, fontSize = 12.sp),
                    color = FiendColors.muted,
                    maxLines = 1,
                )
            }
            profile?.latencyMs?.let { latency ->
                Text(
                    text = stringResource(R.string.home_latency, latency),
                    style = MaterialTheme.typography.labelMedium,
                    color = latencyColor(latency),
                )
                Spacer(Modifier.width(8.dp))
            }
            Icon(
                FiendIcons.KeyboardArrowDown,
                contentDescription = stringResource(R.string.home_change_server),
                tint = FiendColors.muted,
                modifier = Modifier.size(20.dp),
            )
        }
    }
}

@Composable
private fun StatsGrid(stats: TunnelStats, connected: Boolean) {
    val empty = stringResource(R.string.home_stat_empty)
    val down = formatBitrate(stats.downloadBps)
    val up = formatBitrate(stats.uploadBps)
    val total = formatBytes(stats.receivedBytes + stats.sentBytes)
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            StatTile(
                icon = FiendIcons.ArrowDownward,
                label = stringResource(R.string.home_stat_download),
                value = if (connected) down.value else empty,
                unit = if (connected) down.unit else "",
                modifier = Modifier.weight(1f),
            )
            StatTile(
                icon = FiendIcons.ArrowUpward,
                label = stringResource(R.string.home_stat_upload),
                value = if (connected) up.value else empty,
                unit = if (connected) up.unit else "",
                modifier = Modifier.weight(1f),
            )
        }
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            StatTile(
                icon = FiendIcons.Speed,
                label = stringResource(R.string.home_stat_latency),
                value = if (connected) stats.latencyMs?.toString() ?: empty else empty,
                unit = if (connected && stats.latencyMs != null) stringResource(R.string.home_unit_ms) else "",
                modifier = Modifier.weight(1f),
            )
            StatTile(
                icon = FiendIcons.DataUsage,
                label = stringResource(R.string.home_stat_session),
                value = if (connected) total.value else empty,
                unit = if (connected) total.unit else "",
                modifier = Modifier.weight(1f),
            )
        }
    }
}
