package dev.fiend.feature.settings

import android.content.ActivityNotFoundException
import android.content.Intent
import android.provider.Settings
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.LargeFlexibleTopAppBar
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import dev.fiend.core.designsystem.component.ActionItem
import dev.fiend.core.designsystem.component.InfoItem
import dev.fiend.core.designsystem.component.SegmentedGroup
import dev.fiend.core.designsystem.component.SwitchItem
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.core.designsystem.theme.supportsDynamicColor
import dev.fiend.core.model.AppSettings
import dev.fiend.core.model.SplitTunnelMode
import dev.fiend.core.model.ThemeMode
import kotlinx.coroutines.launch

@Composable
internal fun SettingsScreen(
    settings: AppSettings,
    versionName: String,
    onUpdate: ((AppSettings) -> AppSettings) -> Unit,
    onTheme: (ThemeMode) -> Unit,
    onOpenSplitTunnel: () -> Unit,
    onOpenRouting: () -> Unit,
    onOpenLogs: () -> Unit,
) {
    val scrollBehavior = TopAppBarDefaults.exitUntilCollapsedScrollBehavior()
    val context = LocalContext.current
    val resources = LocalResources.current
    val snackbar = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()
    var themeDialog by rememberSaveable { mutableStateOf(false) }

    Scaffold(
        modifier = Modifier.nestedScroll(scrollBehavior.nestedScrollConnection),
        topBar = {
            LargeFlexibleTopAppBar(
                title = { Text(stringResource(R.string.settings_title)) },
                scrollBehavior = scrollBehavior,
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        LazyColumn(
            contentPadding = PaddingValues(
                start = 16.dp,
                end = 16.dp,
                top = padding.calculateTopPadding(),
                bottom = padding.calculateBottomPadding() + 24.dp,
            ),
            modifier = Modifier.fillMaxSize(),
        ) {
            item {
                SegmentedGroup(title = stringResource(R.string.settings_group_connection)) {
                    item {
                        SwitchItem(
                            shapes = it,
                            headline = stringResource(R.string.settings_connect_on_start),
                            supporting = stringResource(R.string.settings_connect_on_start_body),
                            icon = FiendIcons.Bolt,
                            checked = settings.connectOnStart,
                            onCheckedChange = { v -> onUpdate { s -> s.copy(connectOnStart = v) } },
                        )
                    }
                    item {
                        ActionItem(
                            shapes = it,
                            headline = stringResource(R.string.settings_always_on),
                            supporting = stringResource(R.string.settings_always_on_body),
                            icon = FiendIcons.Shield,
                            trailing = { Icon(FiendIcons.OpenInNew, contentDescription = null) },
                            onClick = {
                                try {
                                    context.startActivity(Intent(Settings.ACTION_VPN_SETTINGS))
                                } catch (_: ActivityNotFoundException) {
                                    context.startActivity(Intent(Settings.ACTION_SETTINGS))
                                }
                            },
                        )
                    }
                }
            }
            item {
                SegmentedGroup(title = stringResource(R.string.settings_group_routing)) {
                    item {
                        ActionItem(
                            shapes = it,
                            headline = stringResource(R.string.settings_split),
                            supporting = when (settings.splitTunnelMode) {
                                SplitTunnelMode.AllApps -> stringResource(R.string.settings_split_all)
                                SplitTunnelMode.OnlySelected -> pluralStringResource(
                                    R.plurals.settings_split_only,
                                    settings.splitTunnelApps.size,
                                    settings.splitTunnelApps.size,
                                )
                                SplitTunnelMode.ExceptSelected -> pluralStringResource(
                                    R.plurals.settings_split_except,
                                    settings.splitTunnelApps.size,
                                    settings.splitTunnelApps.size,
                                )
                            },
                            icon = FiendIcons.AltRoute,
                            trailing = { Icon(FiendIcons.ChevronRight, contentDescription = null) },
                            onClick = onOpenSplitTunnel,
                        )
                    }
                    item {
                        ActionItem(
                            shapes = it,
                            headline = stringResource(R.string.settings_rules),
                            supporting = stringResource(R.string.settings_rules_body),
                            icon = FiendIcons.Block,
                            trailing = { Icon(FiendIcons.ChevronRight, contentDescription = null) },
                            onClick = onOpenRouting,
                        )
                    }
                    item {
                        SwitchItem(
                            shapes = it,
                            headline = stringResource(R.string.settings_bypass_lan),
                            supporting = stringResource(R.string.settings_bypass_lan_body),
                            icon = FiendIcons.Lan,
                            checked = settings.bypassLan,
                            onCheckedChange = { v -> onUpdate { s -> s.copy(bypassLan = v) } },
                        )
                    }
                    item {
                        SwitchItem(
                            shapes = it,
                            headline = stringResource(R.string.settings_ipv6),
                            supporting = stringResource(R.string.settings_ipv6_body),
                            icon = FiendIcons.Language,
                            checked = settings.ipv6,
                            onCheckedChange = { v -> onUpdate { s -> s.copy(ipv6 = v) } },
                        )
                    }
                    item {
                        SwitchItem(
                            shapes = it,
                            headline = stringResource(R.string.settings_remote_dns),
                            supporting = stringResource(R.string.settings_remote_dns_body),
                            icon = FiendIcons.Dns,
                            checked = true,
                            enabled = false,
                            onCheckedChange = {},

                        )
                    }
                }
            }
            item {
                SegmentedGroup(title = stringResource(R.string.settings_group_appearance)) {
                    item {
                        ActionItem(
                            shapes = it,
                            headline = stringResource(R.string.settings_theme),
                            supporting = themeLabel(settings.themeMode),
                            icon = FiendIcons.DarkMode,
                            onClick = { themeDialog = true },
                        )
                    }
                    item {
                        SwitchItem(
                            shapes = it,
                            headline = stringResource(R.string.settings_dynamic_color),
                            supporting = stringResource(
                                if (supportsDynamicColor) R.string.settings_dynamic_color_body else R.string.settings_dynamic_color_unsupported,
                            ),
                            icon = FiendIcons.Palette,
                            enabled = supportsDynamicColor,
                            checked = settings.dynamicColor && supportsDynamicColor,
                            onCheckedChange = { v -> onUpdate { s -> s.copy(dynamicColor = v) } },
                        )
                    }
                }
            }
            item {
                SegmentedGroup(title = stringResource(R.string.settings_group_about)) {
                    item {
                        ActionItem(
                            shapes = it,
                            headline = stringResource(R.string.settings_logs),
                            supporting = stringResource(R.string.settings_logs_body),
                            icon = FiendIcons.Article,
                            trailing = { Icon(FiendIcons.ChevronRight, contentDescription = null) },
                            onClick = onOpenLogs,
                        )
                    }
                    item {
                        ActionItem(
                            shapes = it,
                            headline = stringResource(R.string.settings_licenses),
                            icon = FiendIcons.Policy,
                            onClick = {
                                scope.launch { snackbar.showSnackbar(resources.getString(R.string.settings_licenses_unavailable)) }
                            },
                        )
                    }
                    item {
                        InfoItem(it, stringResource(R.string.settings_version), versionName, icon = FiendIcons.Info)
                    }
                }
            }
        }
    }

    if (themeDialog) {
        ThemeDialog(
            current = settings.themeMode,
            onSelect = {
                onTheme(it)
                themeDialog = false
            },
            onDismiss = { themeDialog = false },
        )
    }
}

@Composable
private fun themeLabel(mode: ThemeMode): String = stringResource(
    when (mode) {
        ThemeMode.System -> R.string.settings_theme_system
        ThemeMode.Light -> R.string.settings_theme_light
        ThemeMode.Dark -> R.string.settings_theme_dark
    },
)

@Composable
private fun ThemeDialog(current: ThemeMode, onSelect: (ThemeMode) -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.settings_theme)) },
        text = {
            Column(Modifier.selectableGroup()) {
                ThemeMode.entries.forEach { mode ->
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(56.dp)
                            .selectable(selected = mode == current, onClick = { onSelect(mode) }, role = Role.RadioButton),
                    ) {
                        RadioButton(selected = mode == current, onClick = null)
                        Spacer(Modifier.padding(start = 16.dp))
                        Text(themeLabel(mode), style = MaterialTheme.typography.bodyLarge)
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.settings_cancel)) }
        },
    )
}
