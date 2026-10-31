package dev.fiend.core.designsystem.component

import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import dev.fiend.core.designsystem.theme.FiendColors

@Composable
fun latencyColor(latencyMs: Int?): Color = when {
    latencyMs == null -> FiendColors.muted
    latencyMs < 180 -> FiendColors.green
    latencyMs < 350 -> FiendColors.text
    else -> FiendColors.danger
}
