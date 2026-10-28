package dev.fiend.feature.profiles.component

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.ListItemShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import dev.fiend.core.designsystem.component.FiendListDefaults
import dev.fiend.core.designsystem.component.ShapeAvatar
import dev.fiend.core.designsystem.component.avatarLabel
import dev.fiend.core.designsystem.component.latencyColor
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.core.model.Profile
import dev.fiend.feature.profiles.R

@Composable
internal fun ProfileListItem(
    profile: Profile,
    shapes: ListItemShapes,
    connected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    highlighted: Boolean = false,
) {
    val colors = if (highlighted) {
        FiendListDefaults.colors(containerColor = MaterialTheme.colorScheme.secondaryContainer)
    } else {
        FiendListDefaults.colors()
    }
    SegmentedListItem(
        onClick = onClick,
        shapes = shapes,
        modifier = modifier,
        colors = colors,
        leadingContent = {
            ShapeAvatar(label = avatarLabel(profile.name, profile.countryCode), seed = profile.id)
        },
        supportingContent = { Text(profile.authority, maxLines = 1) },
        trailingContent = {
            when {
                connected -> ConnectedBadge()
                profile.latencyMs != null -> Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(
                        FiendIcons.SignalCellularAlt,
                        contentDescription = null,
                        tint = latencyColor(profile.latencyMs),
                        modifier = Modifier.padding(end = 4.dp),
                    )
                    Text(
                        text = stringResource(R.string.profiles_latency, profile.latencyMs ?: 0),
                        style = MaterialTheme.typography.labelLarge,
                        color = latencyColor(profile.latencyMs),
                    )
                }
            }
        },
    ) {
        Text(profile.name, style = MaterialTheme.typography.titleMedium)
    }
}

@Composable
private fun ConnectedBadge() {
    val description = stringResource(R.string.profiles_connected)
    Surface(
        color = MaterialTheme.colorScheme.primary,
        contentColor = MaterialTheme.colorScheme.onPrimary,
        shape = CircleShape,
        modifier = Modifier
            .size(36.dp)
            .semantics { contentDescription = description },
    ) {
        Box(contentAlignment = Alignment.Center) {
            Icon(FiendIcons.ShieldFilled, contentDescription = null, modifier = Modifier.size(20.dp))
        }
    }
}
