package dev.fiend.core.designsystem.component

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.toShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.graphics.shapes.RoundedPolygon
import dev.fiend.core.designsystem.theme.googleSansFlex

private val avatarShapes: List<RoundedPolygon>
    get() = listOf(
        MaterialShapes.Cookie9Sided,
        MaterialShapes.Clover4Leaf,
        MaterialShapes.Cookie6Sided,
        MaterialShapes.Sunny,
        MaterialShapes.Pentagon,
        MaterialShapes.Arch,
        MaterialShapes.Gem,
        MaterialShapes.SoftBurst,
        MaterialShapes.Cookie4Sided,
    )

fun avatarLabel(name: String, code: String?): String =
    code?.take(2)?.uppercase()
        ?: name.split(' ', '-', '_').filter { it.isNotBlank() }.take(2).joinToString("") { it.take(1) }.uppercase()
            .ifEmpty { "?" }

@Composable
fun ShapeAvatar(
    label: String,
    seed: String,
    modifier: Modifier = Modifier,
    size: Dp = 40.dp,
) {
    val hash = seed.hashCode().let { if (it == Int.MIN_VALUE) 0 else kotlin.math.abs(it) }
    val polygon = avatarShapes[hash % avatarShapes.size]
    val (container, content) = avatarColors(hash)
    Box(
        modifier = modifier
            .size(size)
            .clip(polygon.toShape())
            .background(container),
        contentAlignment = Alignment.Center,
    ) {
        val fontSize = size.value * 0.34f
        Text(
            text = label,
            color = content,
            fontSize = fontSize.sp,
            fontFamily = googleSansFlex(weight = FontWeight.SemiBold, opticalSize = fontSize, roundness = 60f),
            maxLines = 1,
        )
    }
}

@Composable
private fun avatarColors(hash: Int): Pair<Color, Color> {
    val c = MaterialTheme.colorScheme
    val palette = listOf(
        c.primaryContainer to c.onPrimaryContainer,
        c.tertiaryContainer to c.onTertiaryContainer,
        c.secondaryContainer to c.onSecondaryContainer,
        c.primaryFixedDim to c.onPrimaryFixed,
        c.tertiaryFixedDim to c.onTertiaryFixed,
    )
    return palette[(hash / 7) % palette.size]
}
