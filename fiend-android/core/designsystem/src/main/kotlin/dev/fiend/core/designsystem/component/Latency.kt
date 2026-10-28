package dev.fiend.core.designsystem.component

import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

@Composable
fun latencyColor(latencyMs: Int?): Color = when {
    latencyMs == null -> MaterialTheme.colorScheme.onSurfaceVariant
    latencyMs < 90 -> MaterialTheme.colorScheme.primary
    latencyMs < 180 -> MaterialTheme.colorScheme.tertiary
    else -> MaterialTheme.colorScheme.error
}
