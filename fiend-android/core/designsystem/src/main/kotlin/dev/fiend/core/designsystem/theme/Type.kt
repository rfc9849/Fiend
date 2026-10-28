package dev.fiend.core.designsystem.theme

import androidx.compose.material3.Typography
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontVariation
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.isSpecified
import dev.fiend.core.designsystem.R

private val familyCache = HashMap<FlexAxes, FontFamily>()

private data class FlexAxes(val weight: Int, val opticalSize: Int, val roundness: Int, val width: Int)

fun googleSansFlex(
    weight: FontWeight = FontWeight.Normal,
    opticalSize: Float = 18f,
    roundness: Float = 0f,
    width: Float = 100f,
): FontFamily {
    val axes = FlexAxes(weight.weight, opticalSize.toInt().coerceIn(6, 144), roundness.toInt(), width.toInt())
    return familyCache.getOrPut(axes) {
        FontFamily(
            Font(
                resId = R.font.google_sans_flex,
                weight = weight,
                variationSettings = FontVariation.Settings(
                    FontVariation.weight(axes.weight),
                    FontVariation.width(axes.width.toFloat()),
                    FontVariation.Setting("opsz", axes.opticalSize.toFloat()),
                    FontVariation.Setting("ROND", axes.roundness.toFloat()),
                ),
            ),
        )
    }
}

private fun TextStyle.flex(roundness: Float = 0f): TextStyle {
    val weight = fontWeight ?: FontWeight.Normal
    val size: TextUnit = fontSize
    return copy(
        fontFamily = googleSansFlex(
            weight = weight,
            opticalSize = if (size.isSpecified) size.value else 18f,
            roundness = roundness,
        ),
    )
}

private val base = Typography()

private val emphasized = FontWeight.SemiBold

val FiendTypography = Typography(
    displayLarge = base.displayLarge.flex(),
    displayMedium = base.displayMedium.flex(),
    displaySmall = base.displaySmall.flex(),
    headlineLarge = base.headlineLarge.flex(),
    headlineMedium = base.headlineMedium.flex(),
    headlineSmall = base.headlineSmall.flex(),
    titleLarge = base.titleLarge.flex(),
    titleMedium = base.titleMedium.flex(),
    titleSmall = base.titleSmall.flex(),
    bodyLarge = base.bodyLarge.flex(),
    bodyMedium = base.bodyMedium.flex(),
    bodySmall = base.bodySmall.flex(),
    labelLarge = base.labelLarge.flex(),
    labelMedium = base.labelMedium.flex(),
    labelSmall = base.labelSmall.flex(),
    displayLargeEmphasized = base.displayLargeEmphasized.copy(fontWeight = emphasized).flex(roundness = 40f),
    displayMediumEmphasized = base.displayMediumEmphasized.copy(fontWeight = emphasized).flex(roundness = 40f),
    displaySmallEmphasized = base.displaySmallEmphasized.copy(fontWeight = emphasized).flex(roundness = 40f),
    headlineLargeEmphasized = base.headlineLargeEmphasized.copy(fontWeight = emphasized).flex(roundness = 30f),
    headlineMediumEmphasized = base.headlineMediumEmphasized.copy(fontWeight = emphasized).flex(roundness = 30f),
    headlineSmallEmphasized = base.headlineSmallEmphasized.copy(fontWeight = emphasized).flex(roundness = 30f),
    titleLargeEmphasized = base.titleLargeEmphasized.copy(fontWeight = emphasized).flex(),
    titleMediumEmphasized = base.titleMediumEmphasized.flex(),
    titleSmallEmphasized = base.titleSmallEmphasized.flex(),
    bodyLargeEmphasized = base.bodyLargeEmphasized.flex(),
    bodyMediumEmphasized = base.bodyMediumEmphasized.flex(),
    bodySmallEmphasized = base.bodySmallEmphasized.flex(),
    labelLargeEmphasized = base.labelLargeEmphasized.flex(),
    labelMediumEmphasized = base.labelMediumEmphasized.flex(),
    labelSmallEmphasized = base.labelSmallEmphasized.flex(),
)
