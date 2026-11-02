package dev.fiend.feature.settings

import android.content.ActivityNotFoundException
import android.content.Intent
import android.provider.Settings
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import dev.fiend.core.designsystem.component.ActionItem
import dev.fiend.core.designsystem.component.FiendButton
import dev.fiend.core.designsystem.component.FiendButtonKind
import dev.fiend.core.designsystem.component.FiendGroup
import dev.fiend.core.designsystem.component.FiendTag
import dev.fiend.core.designsystem.component.FiendTagTone
import dev.fiend.core.designsystem.component.FiendTopBar
import dev.fiend.core.designsystem.component.InfoItem
import dev.fiend.core.designsystem.component.SwitchItem
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.core.designsystem.theme.FiendColors
import dev.fiend.core.model.AppSettings
import dev.fiend.core.model.SplitTunnelMode
import dev.fiend.core.model.ThemeMode

@Composable
internal fun SettingsScreen(
    settings: AppSettings,
    versionName: String,
    onUpdate: ((AppSettings) -> AppSettings) -> Unit,
    onTheme: (ThemeMode) -> Unit,
    onOpenSplitTunnel: () -> Unit,
    onOpenRouting: () -> Unit,
) {
    val context = LocalContext.current
    val uriHandler = LocalUriHandler.current
    var themeDialog by rememberSaveable { mutableStateOf(false) }

    Column(
        modifier = Modifier.fillMaxSize(),
    ) {
        FiendTopBar(title = stringResource(R.string.settings_title))
        LazyColumn(
            contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 24.dp),
            modifier = Modifier.weight(1f).fillMaxWidth(),
        ) {
            item {
                FiendGroup(title = stringResource(R.string.settings_group_connection)) {
                    item {
                        SwitchItem(
                            headline = stringResource(R.string.settings_connect_on_start),
                            supporting = stringResource(R.string.settings_connect_on_start_body),
                            icon = FiendIcons.Bolt,
                            checked = settings.connectOnStart,
                            onCheckedChange = { v -> onUpdate { s -> s.copy(connectOnStart = v) } },
                        )
                    }
                    item {
                        ActionItem(
                            headline = stringResource(R.string.settings_always_on),
                            supporting = stringResource(R.string.settings_always_on_body),
                            icon = FiendIcons.Shield,
                            trailing = { Icon(FiendIcons.OpenInNew, contentDescription = null, tint = FiendColors.muted) },
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
                FiendGroup(title = stringResource(R.string.settings_group_routing)) {
                    item {
                        ActionItem(
                            headline = stringResource(R.string.settings_rules),
                            supporting = stringResource(R.string.settings_rules_body),
                            icon = FiendIcons.Policy,
                            trailing = { Icon(FiendIcons.ChevronRight, contentDescription = null, tint = FiendColors.muted) },
                            onClick = onOpenRouting,
                        )
                    }
                    item {
                        ActionItem(
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
                            trailing = { Icon(FiendIcons.ChevronRight, contentDescription = null, tint = FiendColors.muted) },
                            onClick = onOpenSplitTunnel,
                        )
                    }
                    item {
                        SwitchItem(
                            headline = stringResource(R.string.settings_bypass_lan),
                            supporting = stringResource(R.string.settings_bypass_lan_body),
                            icon = FiendIcons.Lan,
                            checked = settings.bypassLan,
                            onCheckedChange = { v -> onUpdate { s -> s.copy(bypassLan = v) } },
                        )
                    }
                    item {
                        SwitchItem(
                            headline = stringResource(R.string.settings_ipv6),
                            supporting = stringResource(R.string.settings_ipv6_body),
                            icon = FiendIcons.Language,
                            checked = settings.ipv6,
                            onCheckedChange = { v -> onUpdate { s -> s.copy(ipv6 = v) } },
                        )
                    }
                }
            }
            item {
                FiendGroup(title = stringResource(R.string.settings_group_appearance)) {
                    item {
                        ActionItem(
                            headline = stringResource(R.string.settings_theme),
                            supporting = themeLabel(settings.themeMode),
                            icon = FiendIcons.DarkMode,
                            trailing = { Icon(FiendIcons.ChevronRight, contentDescription = null, tint = FiendColors.muted) },
                            onClick = { themeDialog = true },
                        )
                    }
                }
            }
            item {
                FiendGroup(title = stringResource(R.string.settings_group_about)) {
                    item {
                        InfoItem(
                            headline = stringResource(R.string.settings_version),
                            value = versionName,
                            icon = FiendIcons.Info,
                        )
                    }
                    item {
                        ActionItem(
                            headline = stringResource(R.string.settings_github),
                            supporting = "rfc9849/Fiend",
                            icon = FiendIcons.Link,
                            trailing = { Icon(FiendIcons.OpenInNew, contentDescription = null, tint = FiendColors.muted) },
                            onClick = { uriHandler.openUri("https://github.com/rfc9849/Fiend") },
                        )
                    }
                    item {
                        ActionItem(
                            headline = stringResource(R.string.settings_license),
                            supporting = stringResource(R.string.settings_license_name),
                            icon = FiendIcons.Article,
                            trailing = { Icon(FiendIcons.OpenInNew, contentDescription = null, tint = FiendColors.muted) },
                            onClick = { uriHandler.openUri("https://github.com/rfc9849/Fiend/blob/master/LICENSE") },
                        )
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
        shape = RoundedCornerShape(8.dp),
        containerColor = FiendColors.panel,
        titleContentColor = FiendColors.text,
        textContentColor = FiendColors.text,
        title = { Text(stringResource(R.string.settings_theme), style = MaterialTheme.typography.titleLarge) },
        text = {
            Column {
                ThemeMode.entries.forEach { mode ->
                    val selected = mode == current
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(48.dp)
                            .clickable { onSelect(mode) }
                            .padding(horizontal = 4.dp),
                    ) {
                        Text(
                            text = themeLabel(mode),
                            style = MaterialTheme.typography.bodyLarge,
                            color = FiendColors.text,
                            modifier = Modifier.weight(1f),
                        )
                        if (selected) {
                            FiendTag(stringResource(R.string.settings_theme_current), FiendTagTone.Green)
                        }
                    }
                }
            }
        },
        confirmButton = {
            FiendButton(
                label = stringResource(R.string.settings_cancel),
                onClick = onDismiss,
                kind = FiendButtonKind.Ghost,
                compact = true,
            )
        },
    )
}
