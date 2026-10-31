package dev.fiend.core.designsystem.theme

import androidx.compose.material3.Typography
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

private val Sans = FontFamily.Default
private val Base = Typography()

private fun TextStyle.ui(
    size: Float,
    line: Float,
    weight: FontWeight = FontWeight.Normal,
    spacing: Float = 0f,
) = copy(
    fontFamily = Sans,
    fontSize = size.sp,
    lineHeight = line.sp,
    fontWeight = weight,
    letterSpacing = spacing.sp,
)

val FiendTypography = Typography(
    displayLarge = Base.displayLarge.ui(36f, 42f, FontWeight.Medium, -0.7f),
    displayMedium = Base.displayMedium.ui(32f, 38f, FontWeight.SemiBold, -0.6f),
    displaySmall = Base.displaySmall.ui(28f, 34f, FontWeight.SemiBold, -0.5f),
    headlineLarge = Base.headlineLarge.ui(28f, 34f, FontWeight.SemiBold, -0.5f),
    headlineMedium = Base.headlineMedium.ui(24f, 30f, FontWeight.Medium, -0.4f),
    headlineSmall = Base.headlineSmall.ui(21f, 28f, FontWeight.SemiBold, -0.3f),
    titleLarge = Base.titleLarge.ui(20f, 26f, FontWeight.SemiBold, -0.2f),
    titleMedium = Base.titleMedium.ui(16f, 22f, FontWeight.Medium),
    titleSmall = Base.titleSmall.ui(14f, 20f, FontWeight.Medium),
    bodyLarge = Base.bodyLarge.ui(16f, 24f),
    bodyMedium = Base.bodyMedium.ui(14f, 20f),
    bodySmall = Base.bodySmall.ui(12f, 18f),
    labelLarge = Base.labelLarge.ui(14f, 20f, FontWeight.Medium),
    labelMedium = Base.labelMedium.ui(12f, 18f, FontWeight.Medium),
    labelSmall = Base.labelSmall.ui(11f, 16f, FontWeight.Medium),
    displayLargeEmphasized = Base.displayLarge.ui(36f, 42f, FontWeight.SemiBold, -0.7f),
    displayMediumEmphasized = Base.displayMedium.ui(32f, 38f, FontWeight.SemiBold, -0.6f),
    displaySmallEmphasized = Base.displaySmall.ui(28f, 34f, FontWeight.SemiBold, -0.5f),
    headlineLargeEmphasized = Base.headlineLarge.ui(28f, 34f, FontWeight.SemiBold, -0.5f),
    headlineMediumEmphasized = Base.headlineMedium.ui(24f, 30f, FontWeight.SemiBold, -0.4f),
    headlineSmallEmphasized = Base.headlineSmall.ui(21f, 28f, FontWeight.SemiBold, -0.3f),
    titleLargeEmphasized = Base.titleLarge.ui(20f, 26f, FontWeight.SemiBold, -0.2f),
    titleMediumEmphasized = Base.titleMedium.ui(16f, 22f, FontWeight.SemiBold),
    titleSmallEmphasized = Base.titleSmall.ui(14f, 20f, FontWeight.SemiBold),
    bodyLargeEmphasized = Base.bodyLarge.ui(16f, 24f, FontWeight.Medium),
    bodyMediumEmphasized = Base.bodyMedium.ui(14f, 20f, FontWeight.Medium),
    bodySmallEmphasized = Base.bodySmall.ui(12f, 18f, FontWeight.Medium),
    labelLargeEmphasized = Base.labelLarge.ui(14f, 20f, FontWeight.SemiBold),
    labelMediumEmphasized = Base.labelMedium.ui(12f, 18f, FontWeight.SemiBold),
    labelSmallEmphasized = Base.labelSmall.ui(11f, 16f, FontWeight.SemiBold),
)
