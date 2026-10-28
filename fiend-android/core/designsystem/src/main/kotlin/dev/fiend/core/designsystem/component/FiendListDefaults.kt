package dev.fiend.core.designsystem.component

import androidx.compose.material3.ListItemColors
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

object FiendListDefaults {
    @Composable
    fun colors(
        containerColor: Color = MaterialTheme.colorScheme.surfaceContainer,
        selectedContainerColor: Color = MaterialTheme.colorScheme.secondaryContainer,
    ): ListItemColors = ListItemDefaults.segmentedColors(
        containerColor = containerColor,
        selectedContainerColor = selectedContainerColor,
        disabledContainerColor = containerColor.copy(alpha = 0.6f),
    )
}
