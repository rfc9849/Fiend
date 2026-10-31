package dev.fiend.feature.profiles.component

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.fiend.core.designsystem.component.FiendTag
import dev.fiend.core.designsystem.component.FiendTagTone
import dev.fiend.core.designsystem.component.ShapeAvatar
import dev.fiend.core.designsystem.component.avatarLabel
import dev.fiend.core.designsystem.component.latencyColor
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.core.designsystem.theme.FiendColors
import dev.fiend.core.model.Profile
import dev.fiend.feature.profiles.R

@Composable
internal fun ProfileListItem(
    profile: Profile,
    connected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    highlighted: Boolean = false,
) {
    Row(
        modifier = modifier
            .fillMaxWidth()
            .heightIn(min = 72.dp)
            .background(if (highlighted) FiendColors.accentSoft else FiendColors.panel)
            .clickable(onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        ShapeAvatar(label = avatarLabel(profile.name, profile.countryCode), seed = profile.id)
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(
                text = profile.name,
                style = MaterialTheme.typography.titleSmall,
                color = FiendColors.text,
                maxLines = 1,
            )
            Text(
                text = profile.authority,
                style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace, fontSize = 12.sp),
                color = FiendColors.muted,
                maxLines = 1,
            )
        }
        when {
            connected -> FiendTag(stringResource(R.string.profiles_connected), FiendTagTone.Green)
            else -> {
                val latency = profile.latencyMs
                if (latency != null) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Icon(
                            FiendIcons.SignalCellularAlt,
                            contentDescription = null,
                            tint = latencyColor(latency),
                            modifier = Modifier.size(16.dp),
                        )
                        Spacer(Modifier.width(4.dp))
                        Text(
                            text = stringResource(R.string.profiles_latency, latency),
                            style = MaterialTheme.typography.labelMedium,
                            color = latencyColor(latency),
                        )
                    }
                } else {
                    Icon(
                        FiendIcons.ChevronRight,
                        contentDescription = null,
                        tint = FiendColors.muted,
                        modifier = Modifier.size(20.dp),
                    )
                }
            }
        }
    }
}
