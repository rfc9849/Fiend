package dev.fiend.feature.settings

import android.graphics.BitmapFactory
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
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
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CheckboxDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import dev.fiend.core.designsystem.component.FiendButton
import dev.fiend.core.designsystem.component.FiendButtonKind
import dev.fiend.core.designsystem.component.FiendTextField
import dev.fiend.core.designsystem.component.FiendTopBar
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.core.designsystem.theme.FiendColors
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
    val editable = state.mode != SplitTunnelMode.AllApps
    Column(Modifier.fillMaxSize().background(FiendColors.surface)) {
        FiendTopBar(
            title = stringResource(R.string.split_title),
            onBack = onBack,
            backLabel = stringResource(R.string.split_back),
        )
        Column(
            modifier = Modifier
                .weight(1f)
                .fillMaxWidth()
                .padding(start = 16.dp, end = 16.dp, bottom = 16.dp),
        ) {
            ModeSelector(state.mode, onMode)
            Spacer(Modifier.height(12.dp))
            Text(
                text = stringResource(
                    when (state.mode) {
                        SplitTunnelMode.AllApps -> R.string.split_body_all
                        SplitTunnelMode.OnlySelected -> R.string.split_body_only
                        SplitTunnelMode.ExceptSelected -> R.string.split_body_except
                    },
                ),
                style = MaterialTheme.typography.bodySmall,
                color = FiendColors.muted,
            )
            Spacer(Modifier.height(12.dp))
            FiendTextField(
                value = state.query,
                onValueChange = onQueryChange,
                placeholder = stringResource(R.string.split_search),
                leadingIcon = FiendIcons.Search,
                enabled = editable,
            )
            Spacer(Modifier.height(12.dp))
            val shape = RoundedCornerShape(6.dp)
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .weight(1f)
                    .clip(shape)
                    .background(FiendColors.panel)
                    .border(1.dp, FiendColors.line, shape),
            ) {
                LazyColumn(Modifier.fillMaxSize()) {
                    itemsIndexed(state.apps, key = { _, it -> it.packageName }) { index, app ->
                        if (index > 0) HorizontalDivider(color = FiendColors.line, thickness = 1.dp)
                        AppRow(
                            app = app,
                            checked = app.packageName in state.selected,
                            enabled = editable,
                            onToggle = { onToggle(app.packageName) },
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun ModeSelector(mode: SplitTunnelMode, onMode: (SplitTunnelMode) -> Unit) {
    Row(horizontalArrangement = Arrangement.spacedBy(4.dp), modifier = Modifier.fillMaxWidth()) {
        SplitTunnelMode.entries.forEach { option ->
            FiendButton(
                label = stringResource(
                    when (option) {
                        SplitTunnelMode.AllApps -> R.string.split_mode_all
                        SplitTunnelMode.OnlySelected -> R.string.split_mode_only
                        SplitTunnelMode.ExceptSelected -> R.string.split_mode_except
                    },
                ),
                onClick = { onMode(option) },
                kind = if (mode == option) FiendButtonKind.Primary else FiendButtonKind.Default,
                compact = true,
                modifier = Modifier.weight(1f),
            )
        }
    }
}

@Composable
private fun AppRow(app: InstalledApp, checked: Boolean, enabled: Boolean, onToggle: () -> Unit) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 68.dp)
            .clickable(enabled = enabled, role = Role.Checkbox, onClick = onToggle)
            .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        AppIcon(app)
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(
                text = app.label,
                style = MaterialTheme.typography.titleSmall,
                color = FiendColors.text,
                maxLines = 1,
            )
            if (app.isSystem) {
                Text(
                    text = stringResource(R.string.split_system),
                    style = MaterialTheme.typography.bodySmall,
                    color = FiendColors.muted,
                )
            }
        }
        Checkbox(
            checked = checked,
            onCheckedChange = null,
            enabled = enabled,
            colors = CheckboxDefaults.colors(
                checkedColor = FiendColors.accent,
                uncheckedColor = FiendColors.dialEdge,
                checkmarkColor = FiendColors.onAccent,
                disabledCheckedColor = FiendColors.accent.copy(alpha = 0.4f),
                disabledUncheckedColor = FiendColors.dialEdge.copy(alpha = 0.4f),
            ),
        )
    }
}

@Composable
private fun AppIcon(app: InstalledApp) {
    val icon = remember(app.iconPng) { app.iconPng?.let { BitmapFactory.decodeByteArray(it, 0, it.size)?.asImageBitmap() } }
    val shape = RoundedCornerShape(4.dp)
    if (icon != null) {
        Image(icon, contentDescription = null, modifier = Modifier.size(44.dp).clip(shape))
        return
    }
    Box(
        contentAlignment = Alignment.Center,
        modifier = Modifier
            .size(44.dp)
            .clip(shape)
            .background(FiendColors.soft),
    ) {
        Text(
            text = app.label.take(1),
            color = FiendColors.muted,
            style = MaterialTheme.typography.titleSmall,
        )
    }
}
