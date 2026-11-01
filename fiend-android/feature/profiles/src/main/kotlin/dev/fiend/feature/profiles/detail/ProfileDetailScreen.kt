package dev.fiend.feature.profiles.detail

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
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
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.fiend.core.designsystem.component.ActionItem
import dev.fiend.core.designsystem.component.FiendAlertDialog
import dev.fiend.core.designsystem.component.FiendButton
import dev.fiend.core.designsystem.component.FiendButtonKind
import dev.fiend.core.designsystem.component.FiendGroup
import dev.fiend.core.designsystem.component.FiendIconButton
import dev.fiend.core.designsystem.component.FiendTag
import dev.fiend.core.designsystem.component.FiendTagTone
import dev.fiend.core.designsystem.component.FiendTopBar
import dev.fiend.core.designsystem.component.InfoItem
import dev.fiend.core.designsystem.component.SegmentedGroup
import dev.fiend.core.designsystem.component.ShapeAvatar
import dev.fiend.core.designsystem.component.avatarLabel
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.core.designsystem.theme.FiendColors
import dev.fiend.core.model.Profile
import dev.fiend.feature.profiles.R

@Composable
internal fun ProfileDetailRoute(
    viewModel: ProfileDetailViewModel,
    onEdit: () -> Unit,
    onOpenProfile: (String) -> Unit,
    onBack: () -> Unit,
) {
    val state by viewModel.uiState.collectAsStateWithLifecycle()
    val snackbar = remember { SnackbarHostState() }
    val resources = LocalResources.current
    LaunchedEffect(viewModel) {
        viewModel.events.collect { event ->
            when (event) {
                is ProfileDetailEvent.LatencyMeasured ->
                    snackbar.showSnackbar(resources.getString(R.string.detail_test_result, event.name, event.ms))
                is ProfileDetailEvent.Duplicated -> onOpenProfile(event.id)
                ProfileDetailEvent.Deleted -> onBack()
                is ProfileDetailEvent.Failed -> snackbar.showSnackbar(event.reason)
            }
        }
    }
    LaunchedEffect(state.loaded, state.profile) {
        if (state.loaded && state.profile == null) onBack()
    }
    ProfileDetailScreen(
        state = state,
        snackbar = snackbar,
        onBack = onBack,
        onToggleConnection = viewModel::toggleConnection,
        onTest = viewModel::test,
        onEdit = onEdit,
        onDuplicate = viewModel::duplicate,
        onDelete = viewModel::delete,
        shareLink = viewModel::shareLink,
    )
}

@Composable
internal fun ProfileDetailScreen(
    state: ProfileDetailUiState,
    snackbar: SnackbarHostState,
    onBack: () -> Unit,
    onToggleConnection: () -> Unit,
    onTest: () -> Unit,
    onEdit: () -> Unit,
    onDuplicate: () -> Unit,
    onDelete: () -> Unit,
    shareLink: () -> String,
) {
    var menuOpen by remember { mutableStateOf(false) }
    var confirmDelete by rememberSaveable { mutableStateOf(false) }
    var shareOpen by remember { mutableStateOf(false) }
    var sharedLink by remember { mutableStateOf("") }
    var secretVisible by rememberSaveable { mutableStateOf(false) }
    val profile = state.profile

    Box(Modifier.fillMaxSize().background(FiendColors.surface)) {
        Column(Modifier.fillMaxSize()) {
            FiendTopBar(
                onBack = onBack,
                backLabel = stringResource(R.string.detail_back),
                actions = {
                    Box {
                        FiendIconButton(
                            icon = FiendIcons.MoreVert,
                            label = stringResource(R.string.detail_more),
                            onClick = { menuOpen = true },
                        )
                        DropdownMenu(
                            expanded = menuOpen,
                            onDismissRequest = { menuOpen = false },
                            containerColor = FiendColors.panel,
                            shape = RoundedCornerShape(7.dp),
                        ) {
                            DropdownMenuItem(
                                text = { Text(stringResource(R.string.detail_duplicate), style = MaterialTheme.typography.bodyMedium) },
                                leadingIcon = { Icon(FiendIcons.ContentCopy, contentDescription = null, tint = FiendColors.muted) },
                                onClick = {
                                    menuOpen = false
                                    onDuplicate()
                                },
                            )
                            DropdownMenuItem(
                                text = {
                                    Text(
                                        stringResource(R.string.detail_delete),
                                        style = MaterialTheme.typography.bodyMedium,
                                        color = FiendColors.danger,
                                    )
                                },
                                leadingIcon = { Icon(FiendIcons.Delete, contentDescription = null, tint = FiendColors.danger) },
                                onClick = {
                                    menuOpen = false
                                    confirmDelete = true
                                },
                            )
                        }
                    }
                },
            )
            if (profile != null) {
                Column(
                    modifier = Modifier
                        .weight(1f)
                        .fillMaxWidth()
                        .verticalScroll(rememberScrollState())
                        .padding(horizontal = 16.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Column(
                        Modifier
                            .widthIn(max = 720.dp)
                            .padding(top = 8.dp, bottom = 32.dp),
                    ) {
                        Header(profile, state)
                        Spacer(Modifier.height(20.dp))
                        ActionRow(
                            state = state,
                            onToggleConnection = onToggleConnection,
                            onShare = { sharedLink = shareLink(); shareOpen = sharedLink.isNotBlank() },
                            onTest = onTest,
                            onEdit = onEdit,
                        )
                        Spacer(Modifier.height(10.dp))
                        ServerGroup(
                            profile = profile,
                            templateLabel = state.template?.label ?: profile.templateId,
                            secretVisible = secretVisible,
                            onToggleSecret = { secretVisible = !secretVisible },
                        )
                        TuningGroup(profile)
                        Spacer(Modifier.height(20.dp))
                        FiendGroup {
                            item {
                                ActionItem(
                                    headline = stringResource(R.string.detail_delete_profile),
                                    icon = FiendIcons.Delete,
                                    headlineColor = FiendColors.danger,
                                    onClick = { confirmDelete = true },
                                )
                            }
                        }
                    }
                }
            }
        }
        SnackbarHost(
            hostState = snackbar,
            modifier = Modifier
                .align(Alignment.BottomCenter)
                .padding(bottom = 16.dp),
        )
    }

    if (confirmDelete && profile != null) {
        FiendAlertDialog(
            title = stringResource(R.string.detail_delete_title, profile.name),
            confirmLabel = stringResource(R.string.detail_delete),
            onConfirm = {
                confirmDelete = false
                onDelete()
            },
            confirmDanger = true,
            dismissLabel = stringResource(R.string.profiles_cancel),
            onDismiss = { confirmDelete = false },
        ) {
            Text(
                stringResource(R.string.detail_delete_body),
                style = MaterialTheme.typography.bodyMedium,
                color = FiendColors.muted,
            )
        }
    }

    if (shareOpen && profile != null) {
        ShareSheet(name = profile.name, link = sharedLink, onDismiss = { shareOpen = false })
    }
}

@Composable
private fun Header(profile: Profile, state: ProfileDetailUiState) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        ShapeAvatar(
            label = avatarLabel(profile.name, profile.countryCode),
            seed = profile.id,
            size = 60.dp,
        )
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            Text(
                text = profile.name,
                style = MaterialTheme.typography.headlineMedium,
                color = FiendColors.text,
                maxLines = 2,
            )
            Text(
                text = profile.authority,
                style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace, fontSize = 12.sp),
                color = FiendColors.muted,
            )
        }
        if (state.isActive) {
            FiendTag(
                text = stringResource(if (state.isConnected) R.string.profiles_connected else R.string.detail_active),
                tone = if (state.isConnected) FiendTagTone.Green else FiendTagTone.Neutral,
            )
        }
    }
}

@Composable
private fun ActionRow(
    state: ProfileDetailUiState,
    onToggleConnection: () -> Unit,
    onShare: () -> Unit,
    onTest: () -> Unit,
    onEdit: () -> Unit,
) {
    val on = state.isConnected || state.isBusy
    FlowRow(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        FiendButton(
            label = stringResource(if (on) R.string.detail_action_disconnect else R.string.detail_action_connect),
            onClick = onToggleConnection,
            icon = FiendIcons.PowerSettingsNew,
            kind = if (on) FiendButtonKind.Default else FiendButtonKind.Primary,
            busy = state.isBusy,
            busyBlocksClick = false,
        )
        FiendButton(
            label = stringResource(R.string.detail_action_share),
            onClick = onShare,
            icon = FiendIcons.Share,
        )
        FiendButton(
            label = stringResource(R.string.detail_action_test),
            onClick = onTest,
            icon = FiendIcons.Speed,
            busy = state.testing,
        )
        FiendButton(
            label = stringResource(R.string.detail_action_edit),
            onClick = onEdit,
            icon = FiendIcons.Edit,
        )
    }
}

@Composable
private fun ServerGroup(profile: Profile, templateLabel: String, secretVisible: Boolean, onToggleSecret: () -> Unit) {
    SegmentedGroup(title = stringResource(R.string.detail_group_server)) {
        item { InfoItem(stringResource(R.string.detail_address), profile.host, icon = FiendIcons.Public) }
        item { InfoItem(stringResource(R.string.detail_port), profile.port.toString(), icon = FiendIcons.Hub) }
        if (profile.moreServers.isNotEmpty()) {
            item { InfoItem(stringResource(R.string.detail_more_servers), profile.moreServers.joinToString(", "), icon = FiendIcons.Public) }
        }
        item { InfoItem(stringResource(R.string.detail_fingerprint), templateLabel, icon = FiendIcons.Fingerprint) }
        item {
            InfoItem(
                headline = stringResource(R.string.detail_secret),
                value = if (secretVisible) {
                    profile.secret.chunked(8).joinToString(" ")
                } else {
                    stringResource(R.string.detail_secret_hidden, profile.secretHint)
                },
                icon = FiendIcons.Key,
                trailing = {
                    FiendIconButton(
                        icon = if (secretVisible) FiendIcons.VisibilityOff else FiendIcons.Visibility,
                        label = stringResource(if (secretVisible) R.string.detail_hide_secret else R.string.detail_show_secret),
                        onClick = onToggleSecret,
                    )
                },
            )
        }
        item {
            InfoItem(
                stringResource(R.string.detail_latency),
                profile.latencyMs?.let { ms -> stringResource(R.string.profiles_latency, ms) }
                    ?: stringResource(R.string.detail_latency_unknown),
                icon = FiendIcons.Speed,
            )
        }
    }
}

@Composable
private fun TuningGroup(profile: Profile) {
    val t = profile.tuning
    SegmentedGroup(title = stringResource(R.string.detail_group_tuning)) {
        item { InfoItem(stringResource(R.string.detail_handshake), seconds(t.handshakeTimeoutMs), icon = FiendIcons.Timer) }
        item { InfoItem(stringResource(R.string.detail_connections), t.maxConnections.toString(), icon = FiendIcons.Devices) }
        item { InfoItem(stringResource(R.string.detail_udp_idle), seconds(t.udpIdleMs), icon = FiendIcons.Schedule) }
        item { InfoItem(stringResource(R.string.detail_udp_flows), t.maxUdpFlows.toString(), icon = FiendIcons.Memory) }
    }
}

@Composable
private fun seconds(ms: Int): String {
    val locale = androidx.compose.ui.text.intl.Locale.current.platformLocale
    val value = if (ms % 1000 == 0) "${ms / 1000}" else String.format(locale, "%.1f", ms / 1000f)
    return stringResource(R.string.detail_seconds, value)
}

internal val MonoFamily = FontFamily.Monospace
