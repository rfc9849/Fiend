package dev.fiend.feature.home

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearWavyProgressIndicator
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.center
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.fiend.core.designsystem.component.FiendListDefaults
import dev.fiend.core.designsystem.component.ShapeAvatar
import dev.fiend.core.designsystem.component.StatTile
import dev.fiend.core.designsystem.component.avatarLabel
import dev.fiend.core.designsystem.component.formatBitrate
import dev.fiend.core.designsystem.component.formatBytes
import dev.fiend.core.designsystem.component.formatElapsed
import dev.fiend.core.designsystem.component.latencyColor
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.core.designsystem.theme.googleSansFlex
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
    Scaffold(
        topBar = {
            CenterAlignedTopAppBar(
                title = { Wordmark() },
                actions = {
                    IconButton(onClick = onOpenLogs) {
                        Icon(FiendIcons.Article, contentDescription = stringResource(R.string.home_logs))
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = Color.Transparent),
            )
        },
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(padding)
                .padding(horizontal = 16.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Column(Modifier.widthIn(max = 560.dp)) {
                HeroPanel(state = state, onToggle = onToggle)
                Spacer(Modifier.height(12.dp))
                ServerCard(
                    profile = state.activeProfile,
                    onClick = { pickerOpen = true },
                )
                Spacer(Modifier.height(12.dp))
                StatsGrid(state.stats, connected = state.tunnel is TunnelState.Connected)
                Spacer(Modifier.height(24.dp))
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

@Composable
private fun Wordmark() {
    Text(
        text = stringResource(R.string.home_brand),
        fontFamily = googleSansFlex(weight = FontWeight.SemiBold, opticalSize = 28f, roundness = 100f),
        fontSize = 26.sp,
        color = MaterialTheme.colorScheme.onSurface,
    )
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
    val colors = MaterialTheme.colorScheme
    val motion = MaterialTheme.motionScheme
    val phase = state.tunnel.phase
    val panel by animateColorAsState(
        targetValue = when (phase) {
            ConnectPhase.Active -> colors.primaryContainer
            ConnectPhase.Error -> colors.errorContainer
            else -> colors.surfaceContainer
        },
        animationSpec = motion.slowEffectsSpec(),
        label = "panel",
    )
    val onPanel by animateColorAsState(
        targetValue = when (phase) {
            ConnectPhase.Active -> colors.onPrimaryContainer
            ConnectPhase.Error -> colors.onErrorContainer
            else -> colors.onSurface
        },
        animationSpec = motion.slowEffectsSpec(),
        label = "onPanel",
    )
    val glow by animateColorAsState(
        targetValue = if (phase == ConnectPhase.Active) colors.primary.copy(alpha = 0.32f) else Color.Transparent,
        animationSpec = motion.slowEffectsSpec(),
        label = "glow",
    )
    val pulse = if (phase == ConnectPhase.Active) {
        rememberInfiniteTransition(label = "pulse").animateFloat(
            initialValue = 0.82f,
            targetValue = 1f,
            animationSpec = infiniteRepeatable(tween(2_400), RepeatMode.Reverse),
            label = "pulseScale",
        ).value
    } else {
        1f
    }

    Surface(
        color = panel,
        contentColor = onPanel,
        shape = MaterialTheme.shapes.extraExtraLarge,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            modifier = Modifier.padding(top = 28.dp, bottom = 28.dp, start = 24.dp, end = 24.dp),
        ) {
            Box(
                contentAlignment = Alignment.Center,
                modifier = Modifier
                    .size(240.dp)
                    .drawBehind {
                        if (glow.alpha > 0f) {
                            drawCircle(
                                brush = Brush.radialGradient(
                                    colors = listOf(glow, Color.Transparent),
                                    center = size.center,
                                    radius = size.minDimension / 2 * pulse,
                                ),
                                radius = size.minDimension / 2,
                            )
                        }
                    },
            ) {
                val description = stringResource(
                    if (phase == ConnectPhase.Idle || phase == ConnectPhase.Error) R.string.home_action_connect else R.string.home_action_disconnect,
                )
                ConnectButton(
                    phase = phase,
                    onClick = onToggle,
                    contentDescription = description,
                    stateDescription = statusTitle(state.tunnel),
                )
            }
            Spacer(Modifier.height(12.dp))
            AnimatedContent(
                targetState = statusTitle(state.tunnel),
                transitionSpec = {
                    (fadeIn(motion.defaultEffectsSpec()) + slideInVertically(motion.defaultSpatialSpec()) { it / 2 })
                        .togetherWith(fadeOut(motion.fastEffectsSpec()) + slideOutVertically(motion.fastSpatialSpec()) { -it / 2 })
                },
                label = "status",
            ) { title ->
                Text(
                    text = title,
                    style = MaterialTheme.typography.headlineMediumEmphasized,
                    textAlign = TextAlign.Center,
                )
            }
            Spacer(Modifier.height(6.dp))
            StatusDetail(state)
            AnimatedVisibility(
                visible = phase == ConnectPhase.Busy,
                enter = fadeIn() + expandVertically(),
                exit = fadeOut() + shrinkVertically(),
            ) {
                LinearWavyProgressIndicator(
                    modifier = Modifier
                        .padding(top = 16.dp)
                        .width(160.dp),
                )
            }
        }
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
private fun StatusDetail(state: HomeUiState) {
    val tunnel = state.tunnel
    val style = MaterialTheme.typography.bodyLarge
    val muted = MaterialTheme.colorScheme.onSurfaceVariant
    when (tunnel) {
        is TunnelState.Connected -> {
            val elapsed by produceState(0L, tunnel.sinceEpochMs) {
                while (true) {
                    value = System.currentTimeMillis() - tunnel.sinceEpochMs
                    delay(1_000)
                }
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    text = formatElapsed(elapsed),
                    style = MaterialTheme.typography.titleLargeEmphasized.copy(fontFeatureSettings = "tnum"),
                )
                state.activeProfile?.let {
                    Text(
                        text = "  ·  " + stringResource(R.string.home_via, it.name),
                        style = style,
                    )
                }
            }
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
            color = muted,
        )
        is TunnelState.Failed -> Text(tunnel.reason, style = style)
        else -> Text(
            text = state.activeProfile?.let { stringResource(R.string.home_hint_off, it.name) }
                ?: stringResource(R.string.home_hint_no_profile),
            style = style,
            color = muted,
            textAlign = TextAlign.Center,
        )
    }
}

@Composable
private fun ServerCard(profile: Profile?, onClick: () -> Unit) {
    SegmentedListItem(
        onClick = onClick,
        shapes = ListItemDefaults.segmentedShapes(index = 0, count = 1),
        colors = FiendListDefaults.colors(),
        leadingContent = {
            if (profile != null) {
                ShapeAvatar(label = avatarLabel(profile.name, profile.countryCode), seed = profile.id, size = 44.dp)
            }
        },
        overlineContent = { Text(stringResource(R.string.home_server)) },
        supportingContent = {
            if (profile != null) {
                Text(profile.host, maxLines = 1)
            }
        },
        trailingContent = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                profile?.latencyMs?.let {
                    Text(
                        text = stringResource(R.string.home_latency, it),
                        style = MaterialTheme.typography.labelLarge,
                        color = latencyColor(it),
                    )
                    Spacer(Modifier.width(8.dp))
                }
                Icon(FiendIcons.UnfoldMore, contentDescription = stringResource(R.string.home_change_server))
            }
        },
    ) {
        Text(
            text = profile?.name ?: stringResource(R.string.home_no_server),
            style = MaterialTheme.typography.titleMedium,
        )
    }
}

@Composable
private fun StatsGrid(stats: TunnelStats, connected: Boolean) {
    val c = MaterialTheme.colorScheme
    val dark = c.surface.luminance() < 0.5f
    val downContainer = if (dark) lerp(c.surfaceContainerHigh, c.secondary, 0.24f) else c.secondaryContainer
    val upContainer = if (dark) lerp(c.surfaceContainerHigh, c.tertiary, 0.24f) else c.tertiaryContainer
    val downContent = if (dark) c.onSurface else c.onSecondaryContainer
    val upContent = if (dark) c.onSurface else c.onTertiaryContainer
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
                containerColor = if (connected) downContainer else c.surfaceContainer,
                contentColor = if (connected) downContent else c.onSurfaceVariant,
                modifier = Modifier.weight(1f),
            )
            StatTile(
                icon = FiendIcons.ArrowUpward,
                label = stringResource(R.string.home_stat_upload),
                value = if (connected) up.value else empty,
                unit = if (connected) up.unit else "",
                containerColor = if (connected) upContainer else c.surfaceContainer,
                contentColor = if (connected) upContent else c.onSurfaceVariant,
                modifier = Modifier.weight(1f),
            )
        }
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            StatTile(
                icon = FiendIcons.Speed,
                label = stringResource(R.string.home_stat_latency),
                value = if (connected) stats.latencyMs?.toString() ?: empty else empty,
                unit = if (connected && stats.latencyMs != null) stringResource(R.string.home_unit_ms) else "",
                containerColor = if (connected) c.surfaceContainerHigh else c.surfaceContainer,
                contentColor = if (connected) c.onSurface else c.onSurfaceVariant,
                modifier = Modifier.weight(1f),
            )
            StatTile(
                icon = FiendIcons.DataUsage,
                label = stringResource(R.string.home_stat_session),
                value = if (connected) total.value else empty,
                unit = if (connected) total.unit else "",
                containerColor = if (connected) c.surfaceContainerHigh else c.surfaceContainer,
                contentColor = if (connected) c.onSurface else c.onSurfaceVariant,
                modifier = Modifier.weight(1f),
            )
        }
    }
}
