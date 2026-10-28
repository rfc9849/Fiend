package dev.fiend.feature.settings

import androidx.compose.animation.AnimatedContent
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.ButtonGroupDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Text
import androidx.compose.material3.ToggleButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import dev.fiend.core.designsystem.component.FiendListDefaults
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.core.model.InstalledApp
import dev.fiend.core.model.SplitTunnelMode

@Composable
internal fun SplitTunnelScreen(
    state: SplitTunnelUiState,
    onBack: () -> Unit,
    onMode: (SplitTunnelMode) -> Unit,
    onToggle: (String) -> Unit,
    onQueryChange: (String) -> Unit,
) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.split_title)) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(FiendIcons.ArrowBack, contentDescription = stringResource(R.string.split_back))
                    }
                },
            )
        },
    ) { padding ->
        val editable = state.mode != SplitTunnelMode.AllApps
        LazyColumn(
            contentPadding = PaddingValues(
                start = 16.dp,
                end = 16.dp,
                top = padding.calculateTopPadding() + 8.dp,
                bottom = padding.calculateBottomPadding() + 24.dp,
            ),
            modifier = Modifier.fillMaxSize(),
        ) {
            item(key = "mode") { ModeSelector(state.mode, onMode) }
            item(key = "body") {
                AnimatedContent(targetState = state.mode, label = "modeBody") { mode ->
                    Text(
                        text = stringResource(
                            when (mode) {
                                SplitTunnelMode.AllApps -> R.string.split_body_all
                                SplitTunnelMode.OnlySelected -> R.string.split_body_only
                                SplitTunnelMode.ExceptSelected -> R.string.split_body_except
                            },
                        ),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 8.dp, vertical = 16.dp),
                    )
                }
            }
            item(key = "search") {
                OutlinedTextField(
                    value = state.query,
                    onValueChange = onQueryChange,
                    placeholder = { Text(stringResource(R.string.split_search)) },
                    leadingIcon = { Icon(FiendIcons.Search, contentDescription = null) },
                    singleLine = true,
                    enabled = editable,
                    shape = CircleShape,
                    colors = OutlinedTextFieldDefaults.colors(
                        unfocusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                        focusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                        disabledContainerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                        unfocusedBorderColor = Color.Transparent,
                        disabledBorderColor = Color.Transparent,
                    ),
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(bottom = 16.dp),
                )
            }
            itemsIndexed(state.apps, key = { _, it -> it.packageName }) { index, app ->
                AppRow(
                    app = app,
                    index = index,
                    count = state.apps.size,
                    checked = app.packageName in state.selected,
                    enabled = editable,
                    onToggle = { onToggle(app.packageName) },
                )
            }
        }
    }
}

@Composable
private fun ModeSelector(mode: SplitTunnelMode, onMode: (SplitTunnelMode) -> Unit) {
    val options = SplitTunnelMode.entries
    Row(
        horizontalArrangement = Arrangement.spacedBy(ButtonGroupDefaults.ConnectedSpaceBetween),
        modifier = Modifier.fillMaxWidth(),
    ) {
        options.forEachIndexed { index, option ->
            ToggleButton(
                checked = mode == option,
                onCheckedChange = { onMode(option) },
                shapes = when (index) {
                    0 -> ButtonGroupDefaults.connectedLeadingButtonShapes()
                    options.lastIndex -> ButtonGroupDefaults.connectedTrailingButtonShapes()
                    else -> ButtonGroupDefaults.connectedMiddleButtonShapes()
                },
                modifier = Modifier
                    .weight(1f)
                    .height(48.dp)
                    .semantics { role = Role.RadioButton },
            ) {
                Text(
                    stringResource(
                        when (option) {
                            SplitTunnelMode.AllApps -> R.string.split_mode_all
                            SplitTunnelMode.OnlySelected -> R.string.split_mode_only
                            SplitTunnelMode.ExceptSelected -> R.string.split_mode_except
                        },
                    ),
                    maxLines = 1,
                )
            }
        }
    }
}

@Composable
private fun AppRow(app: InstalledApp, index: Int, count: Int, checked: Boolean, enabled: Boolean, onToggle: () -> Unit) {
    SegmentedListItem(
        checked = checked,
        onCheckedChange = { onToggle() },
        enabled = enabled,
        shapes = ListItemDefaults.segmentedShapes(index, count),
        colors = FiendListDefaults.colors(),
        leadingContent = { AppIcon(app) },
        supportingContent = if (app.isSystem) {
            { Text(stringResource(R.string.split_system)) }
        } else {
            null
        },
        trailingContent = { Checkbox(checked = checked, onCheckedChange = null, enabled = enabled) },
        modifier = Modifier.padding(bottom = ListItemDefaults.SegmentedGap),
    ) {
        Text(app.label)
    }
}

@Composable
private fun AppIcon(app: InstalledApp) {
    val c = MaterialTheme.colorScheme
    val palette = listOf(
        c.primary to c.onPrimary,
        c.tertiary to c.onTertiary,
        c.secondary to c.onSecondary,
        c.primaryContainer to c.onPrimaryContainer,
        c.tertiaryContainer to c.onTertiaryContainer,
    )
    val (bg, fg) = palette[(app.packageName.hashCode() and 0x7fffffff) % palette.size]
    Box(
        contentAlignment = Alignment.Center,
        modifier = Modifier
            .size(40.dp)
            .clip(MaterialTheme.shapes.medium)
            .background(bg),
    ) {
        Text(app.label.take(1), color = fg, style = MaterialTheme.typography.titleMedium)
    }
}
