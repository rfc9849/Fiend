package dev.fiend.core.designsystem.component

import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import dev.fiend.core.designsystem.theme.FiendColors

@Composable
fun SegmentedGroup(
    modifier: Modifier = Modifier,
    title: String? = null,
    content: FiendGroupScope.() -> Unit,
) = FiendGroup(modifier = modifier, title = title, content = content)

@Composable
fun ActionItem(
    headline: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    supporting: String? = null,
    icon: ImageVector? = null,
    enabled: Boolean = true,
    headlineColor: Color = Color.Unspecified,
    trailing: @Composable (() -> Unit)? = null,
) = FiendRow(
    headline = headline,
    modifier = modifier,
    supporting = supporting,
    icon = icon,
    trailing = trailing,
    onClick = onClick,
    enabled = enabled,
    headlineColor = headlineColor,
    minHeight = 60.dp,
)

@Composable
fun SwitchItem(
    headline: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    supporting: String? = null,
    icon: ImageVector? = null,
    enabled: Boolean = true,
) = FiendRow(
    headline = headline,
    modifier = modifier,
    supporting = supporting,
    icon = icon,
    trailing = { FiendSwitch(checked = checked, onCheckedChange = null, enabled = enabled) },
    onClick = { onCheckedChange(!checked) },
    enabled = enabled,
    minHeight = 60.dp,
)

@Composable
fun InfoRow(
    label: String,
    value: String,
    modifier: Modifier = Modifier,
    icon: ImageVector? = null,
    trailing: @Composable (() -> Unit)? = null,
) {
    Row(
        modifier = modifier
            .fillMaxWidth()
            .heightIn(min = 52.dp)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (icon != null) {
            Icon(icon, contentDescription = null, tint = FiendColors.muted, modifier = Modifier.size(16.dp))
            Spacer(Modifier.width(10.dp))
        }
        Text(
            text = label,
            style = MaterialTheme.typography.bodyMedium,
            color = FiendColors.muted,
            modifier = Modifier.weight(1f),
        )
        Spacer(Modifier.width(12.dp))
        Text(
            text = value,
            style = MaterialTheme.typography.bodyMedium,
            color = FiendColors.text,
            textAlign = androidx.compose.ui.text.style.TextAlign.End,
        )
        if (trailing != null) {
            Spacer(Modifier.width(6.dp))
            trailing()
        }
    }
}

@Composable
fun InfoItem(
    headline: String,
    value: String,
    modifier: Modifier = Modifier,
    icon: ImageVector? = null,
    trailing: @Composable (() -> Unit)? = null,
) = InfoRow(
    label = headline,
    value = value,
    modifier = modifier,
    icon = icon,
    trailing = trailing,
)
