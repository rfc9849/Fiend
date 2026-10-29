package dev.fiend.feature.profiles.detail

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
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.DropdownMenuPopup
import androidx.compose.material3.DropdownMenuGroup
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.MenuDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.fiend.core.designsystem.component.ActionItem
import dev.fiend.core.designsystem.component.InfoItem
import dev.fiend.core.designsystem.component.SegmentedGroup
import dev.fiend.core.designsystem.component.ShapeAvatar
import dev.fiend.core.designsystem.component.avatarLabel
import dev.fiend.core.designsystem.icon.FiendIcons
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

    Scaffold(
        topBar = {
            TopAppBar(
                title = {},
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(FiendIcons.ArrowBack, contentDescription = stringResource(R.string.detail_back))
                    }
                },
                actions = {
                    Box {
                        IconButton(onClick = { menuOpen = true }) {
                            Icon(FiendIcons.MoreVert, contentDescription = stringResource(R.string.detail_more))
                        }
                        DropdownMenuPopup(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
                            DropdownMenuGroup(shapes = MenuDefaults.groupShape(0, 1)) {
                                DropdownMenuItem(
                                    text = { Text(stringResource(R.string.detail_duplicate)) },
                                    leadingIcon = { Icon(FiendIcons.ContentCopy, contentDescription = null) },
                                    onClick = {
                                        menuOpen = false
                                        onDuplicate()
                                    },
                                    shape = MenuDefaults.leadingItemShape,
                                )
                                DropdownMenuItem(
                                    text = { Text(stringResource(R.string.detail_delete)) },
                                    leadingIcon = { Icon(FiendIcons.Delete, contentDescription = null) },
                                    onClick = {
                                        menuOpen = false
                                        confirmDelete = true
                                    },
                                    shape = MenuDefaults.trailingItemShape,
                                )
                            }
                        }
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = Color.Transparent),
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        if (profile == null) return@Scaffold
        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(padding)
                .padding(horizontal = 16.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Column(Modifier.widthIn(max = 560.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Header(profile, state)
                Spacer(Modifier.height(24.dp))
                ActionRow(
                    state = state,
                    onToggleConnection = onToggleConnection,
                    onShare = { sharedLink = shareLink(); shareOpen = sharedLink.isNotBlank() },
                    onTest = onTest,
                    onEdit = onEdit,
                )
                Spacer(Modifier.height(24.dp))
                ServerGroup(
                    profile = profile,
                    templateLabel = state.template?.label ?: profile.templateId,
                    secretVisible = secretVisible,
                    onToggleSecret = { secretVisible = !secretVisible },
                )
                Spacer(Modifier.height(16.dp))
                TuningGroup(profile)
                Spacer(Modifier.height(16.dp))
                SegmentedGroup {
                    item { shapes ->
                        ActionItem(
                            shapes = shapes,
                            headline = stringResource(R.string.detail_delete_profile),
                            icon = FiendIcons.Delete,
                            headlineColor = MaterialTheme.colorScheme.error,
                            onClick = { confirmDelete = true },
                        )
                    }
                }
                Spacer(Modifier.height(32.dp))
            }
        }
    }

    if (confirmDelete && profile != null) {
        AlertDialog(
            onDismissRequest = { confirmDelete = false },
            icon = { Icon(FiendIcons.Delete, contentDescription = null) },
            title = { Text(stringResource(R.string.detail_delete_title, profile.name)) },
            text = { Text(stringResource(R.string.detail_delete_body)) },
            confirmButton = {
                TextButton(
                    onClick = {
                        confirmDelete = false
                        onDelete()
                    },
                    colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error),
                ) { Text(stringResource(R.string.detail_delete)) }
            },
            dismissButton = {
                TextButton(onClick = { confirmDelete = false }) { Text(stringResource(R.string.profiles_cancel)) }
            },
        )
    }

    if (shareOpen && profile != null) {
        ShareSheet(name = profile.name, link = sharedLink, onDismiss = { shareOpen = false })
    }
}

@Composable
private fun Header(profile: Profile, state: ProfileDetailUiState) {
    ShapeAvatar(label = avatarLabel(profile.name, profile.countryCode), seed = profile.id, size = 112.dp)
    Spacer(Modifier.height(16.dp))
    Text(profile.name, style = MaterialTheme.typography.headlineMediumEmphasized, textAlign = TextAlign.Center)
    Spacer(Modifier.height(4.dp))
    Text(
        profile.authority,
        style = MaterialTheme.typography.bodyLarge,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    if (state.isActive) {
        Spacer(Modifier.height(12.dp))
        Surface(
            color = if (state.isConnected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.secondaryContainer,
            shape = MaterialTheme.shapes.extraLarge,
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                modifier = Modifier.padding(horizontal = 12.dp, vertical = 6.dp),
            ) {
                Icon(
                    if (state.isConnected) FiendIcons.ShieldFilled else FiendIcons.RadioButtonChecked,
                    contentDescription = null,
                    modifier = Modifier.size(16.dp),
                )
                Spacer(Modifier.size(6.dp))
                Text(
                    stringResource(if (state.isConnected) R.string.profiles_connected else R.string.detail_active),
                    style = MaterialTheme.typography.labelLarge,
                )
            }
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
    val c = MaterialTheme.colorScheme
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.fillMaxWidth()) {
        val on = state.isConnected || state.isBusy
        PillAction(
            icon = FiendIcons.PowerSettingsNew,
            label = stringResource(if (on) R.string.detail_action_disconnect else R.string.detail_action_connect),
            onClick = onToggleConnection,
            container = if (on) c.primary else c.primaryContainer,
            content = if (on) c.onPrimary else c.onPrimaryContainer,
            busy = state.isBusy,
            modifier = Modifier.weight(1f),
        )
        PillAction(FiendIcons.Share, stringResource(R.string.detail_action_share), onShare, Modifier.weight(1f))
        PillAction(
            icon = FiendIcons.Speed,
            label = stringResource(R.string.detail_action_test),
            onClick = onTest,
            busy = state.testing,
            modifier = Modifier.weight(1f),
        )
        PillAction(FiendIcons.Edit, stringResource(R.string.detail_action_edit), onEdit, Modifier.weight(1f))
    }
}

@Composable
private fun PillAction(
    icon: ImageVector,
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    container: Color = MaterialTheme.colorScheme.secondaryContainer,
    content: Color = MaterialTheme.colorScheme.onSecondaryContainer,
    busy: Boolean = false,
) {
    Column(modifier, horizontalAlignment = Alignment.CenterHorizontally) {
        Surface(
            onClick = onClick,
            color = container,
            contentColor = content,
            shape = MaterialTheme.shapes.extraLarge,
            modifier = Modifier
                .fillMaxWidth()
                .height(64.dp)
                .semantics { contentDescription = label },
        ) {
            Box(contentAlignment = Alignment.Center) {
                if (busy) {
                    LoadingIndicator(color = content, modifier = Modifier.size(32.dp))
                } else {
                    Icon(icon, contentDescription = null)
                }
            }
        }
        Spacer(Modifier.height(8.dp))
        Text(label, style = MaterialTheme.typography.labelLarge, maxLines = 1, modifier = Modifier.clearAndSetSemantics {})
    }
}

@Composable
private fun ServerGroup(profile: Profile, templateLabel: String, secretVisible: Boolean, onToggleSecret: () -> Unit) {
    SegmentedGroup(title = stringResource(R.string.detail_group_server)) {
        item { InfoItem(it, stringResource(R.string.detail_address), profile.host, icon = FiendIcons.Public) }
        item { InfoItem(it, stringResource(R.string.detail_port), profile.port.toString(), icon = FiendIcons.Hub) }
        item { InfoItem(it, stringResource(R.string.detail_fingerprint), templateLabel, icon = FiendIcons.Fingerprint) }
        item { shapes ->
            InfoItem(
                shapes = shapes,
                headline = stringResource(R.string.detail_secret),
                value = if (secretVisible) {
                    profile.secret.chunked(8).joinToString(" ")
                } else {
                    stringResource(R.string.detail_secret_hidden, profile.secretHint)
                },
                icon = FiendIcons.Key,
                trailing = {
                    IconButton(onClick = onToggleSecret) {
                        Icon(
                            if (secretVisible) FiendIcons.VisibilityOff else FiendIcons.Visibility,
                            contentDescription = stringResource(if (secretVisible) R.string.detail_hide_secret else R.string.detail_show_secret),
                        )
                    }
                },
            )
        }
        item {
            InfoItem(
                it,
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
        item { InfoItem(it, stringResource(R.string.detail_handshake), seconds(t.handshakeTimeoutMs), icon = FiendIcons.Timer) }
        item { InfoItem(it, stringResource(R.string.detail_connections), t.maxConnections.toString(), icon = FiendIcons.Devices) }
        item { InfoItem(it, stringResource(R.string.detail_udp_idle), seconds(t.udpIdleMs), icon = FiendIcons.Schedule) }
        item { InfoItem(it, stringResource(R.string.detail_udp_keepalive), seconds(t.udpKeepaliveMs), icon = FiendIcons.History) }
        item { InfoItem(it, stringResource(R.string.detail_udp_flows), t.maxUdpFlows.toString(), icon = FiendIcons.Memory) }
    }
}

@Composable
private fun seconds(ms: Int): String {
    val locale = androidx.compose.ui.text.intl.Locale.current.platformLocale
    val value = if (ms % 1000 == 0) "${ms / 1000}" else String.format(locale, "%.1f", ms / 1000f)
    return stringResource(R.string.detail_seconds, value)
}

internal val MonoFamily = FontFamily.Monospace
