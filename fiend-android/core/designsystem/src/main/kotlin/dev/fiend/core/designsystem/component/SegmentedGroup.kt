package dev.fiend.core.designsystem.component

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.ListItemShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import dev.fiend.core.designsystem.icon.FiendIcons

class SegmentedGroupScope internal constructor() {
    internal val items = mutableListOf<@Composable (ListItemShapes) -> Unit>()

    fun item(content: @Composable (shapes: ListItemShapes) -> Unit) {
        items += content
    }
}

@Composable
fun SegmentedGroup(
    modifier: Modifier = Modifier,
    title: String? = null,
    content: SegmentedGroupScope.() -> Unit,
) {
    val scope = SegmentedGroupScope().apply(content)
    Column(modifier) {
        if (title != null) GroupHeader(title)
        Column(verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap)) {
            val count = scope.items.size
            scope.items.forEachIndexed { index, item ->
                item(ListItemDefaults.segmentedShapes(index = index, count = count))
            }
        }
    }
}

@Composable
fun GroupHeader(text: String, modifier: Modifier = Modifier) {
    Text(
        text = text,
        style = MaterialTheme.typography.labelLarge,
        color = MaterialTheme.colorScheme.primary,
        modifier = modifier
            .padding(start = 16.dp, end = 16.dp, top = 20.dp, bottom = 8.dp)
            .semantics { heading() },
    )
}

@Composable
fun ActionItem(
    shapes: ListItemShapes,
    headline: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    supporting: String? = null,
    icon: ImageVector? = null,
    enabled: Boolean = true,
    headlineColor: Color = Color.Unspecified,
    trailing: @Composable (() -> Unit)? = null,
) {
    SegmentedListItem(
        onClick = onClick,
        shapes = shapes,
        modifier = modifier,
        enabled = enabled,
        colors = FiendListDefaults.colors(),
        leadingContent = icon?.let { { Icon(it, contentDescription = null, tint = headlineColor.takeOrElse()) } },
        supportingContent = supporting?.let { { Text(it) } },
        trailingContent = trailing,
    ) {
        Text(headline, color = headlineColor)
    }
}

@Composable
fun SwitchItem(
    shapes: ListItemShapes,
    headline: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    supporting: String? = null,
    icon: ImageVector? = null,
    enabled: Boolean = true,
) {
    SegmentedListItem(
        checked = checked,
        onCheckedChange = onCheckedChange,
        shapes = shapes,
        modifier = modifier,
        enabled = enabled,
        colors = FiendListDefaults.colors(selectedContainerColor = MaterialTheme.colorScheme.surfaceContainer),
        leadingContent = icon?.let { { Icon(it, contentDescription = null) } },
        supportingContent = supporting?.let { { Text(it) } },
        trailingContent = {
            Switch(
                checked = checked,
                onCheckedChange = null,
                enabled = enabled,
                thumbContent = if (checked) {
                    { Icon(FiendIcons.Check, contentDescription = null, modifier = Modifier.padding(2.dp)) }
                } else {
                    null
                },
                colors = SwitchDefaults.colors(),
            )
        },
    ) {
        Text(headline)
    }
}

@Composable
fun InfoItem(
    shapes: ListItemShapes,
    headline: String,
    value: String,
    modifier: Modifier = Modifier,
    icon: ImageVector? = null,
    trailing: @Composable (() -> Unit)? = null,
) {
    SegmentedListItem(
        shapes = shapes,
        modifier = modifier,
        colors = FiendListDefaults.colors(),
        leadingContent = icon?.let { { Icon(it, contentDescription = null) } },
        supportingContent = { Text(value) },
        trailingContent = trailing,
    ) {
        Text(headline)
    }
}

@Composable
private fun Color.takeOrElse(): Color = if (this == Color.Unspecified) MaterialTheme.colorScheme.onSurfaceVariant else this
