package dev.fiend.core.designsystem.component

import androidx.compose.animation.core.animateDpAsState
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.fiend.core.designsystem.theme.FiendColors

enum class FiendButtonKind { Default, Primary, Ghost, Danger }

enum class FiendTagTone { Neutral, Green, Danger }

@Composable
fun FiendButton(
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    icon: ImageVector? = null,
    kind: FiendButtonKind = FiendButtonKind.Default,
    enabled: Boolean = true,
    busy: Boolean = false,
    busyBlocksClick: Boolean = true,
    compact: Boolean = false,
) {
    val shape = RoundedCornerShape(4.dp)
    val background = when (kind) {
        FiendButtonKind.Default -> FiendColors.panel
        FiendButtonKind.Primary -> FiendColors.accent
        FiendButtonKind.Ghost -> Color.Transparent
        FiendButtonKind.Danger -> FiendColors.panel
    }
    val content = when (kind) {
        FiendButtonKind.Default -> FiendColors.text
        FiendButtonKind.Primary -> FiendColors.onAccent
        FiendButtonKind.Ghost -> FiendColors.muted
        FiendButtonKind.Danger -> FiendColors.danger
    }
    val border = when (kind) {
        FiendButtonKind.Primary -> FiendColors.accent
        FiendButtonKind.Ghost -> Color.Transparent
        else -> FiendColors.line
    }
    Row(
        modifier = modifier
            .heightIn(min = if (compact) 34.dp else 40.dp)
            .clip(shape)
            .background(background)
            .border(BorderStroke(1.dp, border), shape)
            .clickable(enabled = enabled && (!busy || !busyBlocksClick), role = Role.Button, onClick = onClick)
            .semantics { contentDescription = label }
            .padding(horizontal = if (compact) 12.dp else 16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.Center,
    ) {
        if (busy) {
            CircularProgressIndicator(
                modifier = Modifier.size(16.dp),
                color = content,
                strokeWidth = 2.dp,
            )
        } else if (icon != null) {
            Icon(icon, contentDescription = null, tint = content, modifier = Modifier.size(17.dp))
        }
        if (busy || icon != null) Spacer(Modifier.width(8.dp))
        Text(
            text = label,
            style = if (compact) MaterialTheme.typography.labelMedium else MaterialTheme.typography.labelLarge,
            color = content,
            fontWeight = FontWeight.Medium,
            maxLines = 1,
        )
    }
}

@Composable
fun FiendIconButton(
    icon: ImageVector,
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    tint: Color = FiendColors.muted,
) {
    val shape = RoundedCornerShape(4.dp)
    Box(
        modifier = modifier
            .size(40.dp)
            .clip(shape)
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, contentDescription = null, tint = tint, modifier = Modifier.size(20.dp))
    }
}

@Composable
fun FiendSwitch(
    checked: Boolean,
    onCheckedChange: ((Boolean) -> Unit)?,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    label: String? = null,
) {
    val shape = RoundedCornerShape(12.dp)
    val thumbOffset by animateDpAsState(if (checked) 16.dp else 0.dp, label = "thumb")
    val toggle = if (onCheckedChange != null) {
        Modifier.toggleable(
            value = checked,
            enabled = enabled,
            role = Role.Switch,
            onValueChange = onCheckedChange,
        )
    } else {
        Modifier.semantics { role = Role.Switch }
    }
    Box(
        modifier = modifier
            .width(40.dp)
            .height(24.dp)
            .clip(shape)
            .background(if (checked) FiendColors.accent else FiendColors.line)
            .then(toggle)
            .then(if (label != null) Modifier.semantics { contentDescription = label } else Modifier)
            .padding(2.dp),
    ) {
        Box(
            Modifier
                .offset(x = thumbOffset)
                .size(20.dp)
                .clip(RoundedCornerShape(10.dp))
                .background(if (checked) FiendColors.onAccent else FiendColors.panel),
        )
    }
}

@Composable
fun FiendTag(text: String, tone: FiendTagTone = FiendTagTone.Neutral, modifier: Modifier = Modifier) {
    val (background, content, border) = when (tone) {
        FiendTagTone.Neutral -> Triple(FiendColors.soft, FiendColors.muted, FiendColors.line)
        FiendTagTone.Green -> Triple(FiendColors.greenSoft, FiendColors.green, Color.Transparent)
        FiendTagTone.Danger -> Triple(FiendColors.dangerSoft, FiendColors.danger, Color.Transparent)
    }
    Box(
        modifier = modifier
            .clip(RoundedCornerShape(3.dp))
            .background(background)
            .border(BorderStroke(1.dp, border), RoundedCornerShape(3.dp))
            .padding(horizontal = 8.dp, vertical = 4.dp),
    ) {
        Text(text, style = MaterialTheme.typography.labelSmall, color = content, maxLines = 1)
    }
}

@Composable
fun FiendSectionTitle(text: String, modifier: Modifier = Modifier) {
    Text(
        text = text,
        style = MaterialTheme.typography.labelMedium,
        color = FiendColors.muted,
        modifier = modifier
            .padding(start = 4.dp, top = 24.dp, bottom = 8.dp)
            .semantics { heading() },
    )
}

class FiendGroupScope internal constructor() {
    internal val items = mutableListOf<@Composable () -> Unit>()

    fun item(content: @Composable () -> Unit) {
        items += content
    }
}

@Composable
fun FiendGroup(
    modifier: Modifier = Modifier,
    title: String? = null,
    content: FiendGroupScope.() -> Unit,
) {
    val scope = FiendGroupScope().apply(content)
    val shape = RoundedCornerShape(6.dp)
    Column(modifier.fillMaxWidth()) {
        if (title != null) FiendSectionTitle(title)
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .clip(shape)
                .background(FiendColors.panel)
                .border(1.dp, FiendColors.line, shape),
        ) {
            scope.items.forEachIndexed { index, item ->
                if (index > 0) HorizontalDivider(color = FiendColors.line, thickness = 1.dp)
                item()
            }
        }
    }
}

@Composable
fun FiendRow(
    headline: String,
    modifier: Modifier = Modifier,
    supporting: String? = null,
    icon: ImageVector? = null,
    trailing: @Composable (() -> Unit)? = null,
    onClick: (() -> Unit)? = null,
    enabled: Boolean = true,
    headlineColor: Color = Color.Unspecified,
    minHeight: Dp = 56.dp,
) {
    val click = if (onClick != null) {
        Modifier.clickable(enabled = enabled, role = Role.Button, onClick = onClick)
    } else {
        Modifier
    }
    Row(
        modifier = modifier
            .fillMaxWidth()
            .heightIn(min = minHeight)
            .then(click)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (icon != null) {
            Icon(icon, contentDescription = null, tint = FiendColors.muted, modifier = Modifier.size(18.dp))
            Spacer(Modifier.width(12.dp))
        }
        Column(Modifier.weight(1f)) {
            Text(
                text = headline,
                style = MaterialTheme.typography.titleSmall,
                color = if (headlineColor == Color.Unspecified) FiendColors.text else headlineColor,
            )
            if (supporting != null) {
                Text(
                    text = supporting,
                    style = MaterialTheme.typography.bodySmall,
                    color = FiendColors.muted,
                    maxLines = 3,
                )
            }
        }
        if (trailing != null) {
            Spacer(Modifier.width(12.dp))
            trailing()
        }
    }
}

@Composable
fun FiendTextField(
    value: String,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    label: String? = null,
    placeholder: String? = null,
    leadingIcon: ImageVector? = null,
    trailing: @Composable (() -> Unit)? = null,
    supporting: String? = null,
    isError: Boolean = false,
    singleLine: Boolean = true,
    enabled: Boolean = true,
    readOnly: Boolean = false,
    keyboardOptions: KeyboardOptions = KeyboardOptions.Default,
    visualTransformation: VisualTransformation = VisualTransformation.None,
    textStyle: androidx.compose.ui.text.TextStyle = MaterialTheme.typography.bodyMedium,
) {
    OutlinedTextField(
        value = value,
        onValueChange = onValueChange,
        modifier = modifier.fillMaxWidth(),
        enabled = enabled,
        readOnly = readOnly,
        textStyle = textStyle,
        label = label?.let { { Text(it, style = MaterialTheme.typography.labelMedium) } },
        placeholder = placeholder?.let { { Text(it, style = MaterialTheme.typography.bodyMedium, color = FiendColors.muted) } },
        leadingIcon = leadingIcon?.let { { Icon(it, contentDescription = null, tint = FiendColors.muted, modifier = Modifier.size(20.dp)) } },
        trailingIcon = trailing,
        supportingText = supporting?.let { { Text(it, style = MaterialTheme.typography.bodySmall) } },
        isError = isError,
        singleLine = singleLine,
        visualTransformation = visualTransformation,
        keyboardOptions = keyboardOptions,
        shape = RoundedCornerShape(6.dp),
        colors = OutlinedTextFieldDefaults.colors(
            focusedBorderColor = FiendColors.accent,
            unfocusedBorderColor = FiendColors.line,
            disabledBorderColor = FiendColors.line.copy(alpha = 0.6f),
            errorBorderColor = FiendColors.danger,
            focusedContainerColor = FiendColors.panel,
            unfocusedContainerColor = FiendColors.panel,
            disabledContainerColor = FiendColors.soft,
            errorContainerColor = FiendColors.panel,
            cursorColor = FiendColors.accent,
            focusedTextColor = FiendColors.text,
            unfocusedTextColor = FiendColors.text,
            disabledTextColor = FiendColors.muted,
            focusedLabelColor = FiendColors.accent,
            unfocusedLabelColor = FiendColors.muted,
            disabledLabelColor = FiendColors.muted,
            errorLabelColor = FiendColors.danger,
            focusedLeadingIconColor = FiendColors.muted,
            unfocusedLeadingIconColor = FiendColors.muted,
            focusedTrailingIconColor = FiendColors.muted,
            unfocusedTrailingIconColor = FiendColors.muted,
            focusedSupportingTextColor = FiendColors.muted,
            unfocusedSupportingTextColor = FiendColors.muted,
            errorSupportingTextColor = FiendColors.danger,
        ),
    )
}

@Composable
fun FiendAlertDialog(
    title: String,
    confirmLabel: String,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
    confirmEnabled: Boolean = true,
    confirmDanger: Boolean = false,
    dismissLabel: String? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        modifier = modifier,
        shape = RoundedCornerShape(8.dp),
        containerColor = FiendColors.panel,
        titleContentColor = FiendColors.text,
        textContentColor = FiendColors.text,
        title = { Text(title, style = MaterialTheme.typography.titleLarge) },
        text = { Column(content = content) },
        confirmButton = {
            FiendButton(
                label = confirmLabel,
                onClick = onConfirm,
                kind = if (confirmDanger) FiendButtonKind.Danger else FiendButtonKind.Primary,
                enabled = confirmEnabled,
                compact = true,
            )
        },
        dismissButton = dismissLabel?.let {
            {
                FiendButton(
                    label = it,
                    onClick = onDismiss,
                    kind = FiendButtonKind.Ghost,
                    compact = true,
                )
            }
        },
    )
}
