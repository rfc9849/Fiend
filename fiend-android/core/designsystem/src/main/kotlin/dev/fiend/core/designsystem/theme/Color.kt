package dev.fiend.core.designsystem.theme

import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color

@Immutable
data class FiendPalette(
    val canvas: Color,
    val surface: Color,
    val sidebar: Color,
    val panel: Color,
    val soft: Color,
    val hover: Color,
    val line: Color,
    val text: Color,
    val muted: Color,
    val accent: Color,
    val accentHover: Color,
    val onAccent: Color,
    val accentSoft: Color,
    val hero: Color,
    val green: Color,
    val greenSoft: Color,
    val danger: Color,
    val dangerSoft: Color,
    val dialEdge: Color,
    val dialSurface: Color,
    val scrim: Color,
)

val FiendLightPalette = FiendPalette(
    canvas = Color(0xFFE3E5E6),
    surface = Color(0xFFFAFBFB),
    sidebar = Color(0xFFEFF1F2),
    panel = Color(0xFFFFFFFF),
    soft = Color(0xFFF3F5F6),
    hover = Color(0xFFE9EDF0),
    line = Color(0xFFD9DFE3),
    text = Color(0xFF242B30),
    muted = Color(0xFF677179),
    accent = Color(0xFF365F87),
    accentHover = Color(0xFF2A4D70),
    onAccent = Color(0xFFFFFFFF),
    accentSoft = Color(0xFFE6EDF3),
    hero = Color(0xFFFFFFFF),
    green = Color(0xFF24735C),
    greenSoft = Color(0xFFEDF5F1),
    danger = Color(0xFFAE3944),
    dangerSoft = Color(0xFFF8ECEE),
    dialEdge = Color(0xFFBDC7CD),
    dialSurface = Color(0xFFFFFFFF),
    scrim = Color(0x80080C10),
)

val FiendDarkPalette = FiendPalette(
    canvas = Color(0xFF101214),
    surface = Color(0xFF1B1E21),
    sidebar = Color(0xFF171A1D),
    panel = Color(0xFF22262A),
    soft = Color(0xFF202428),
    hover = Color(0xFF2A3035),
    line = Color(0xFF353D43),
    text = Color(0xFFE2E7EB),
    muted = Color(0xFFA0AAB3),
    accent = Color(0xFFA0BDD6),
    accentHover = Color(0xFFB9CEDE),
    onAccent = Color(0xFF172735),
    accentSoft = Color(0xFF293946),
    hero = Color(0xFF1E2226),
    green = Color(0xFF8EC5B0),
    greenSoft = Color(0xFF263B32),
    danger = Color(0xFFE8999E),
    dangerSoft = Color(0xFF3B2729),
    dialEdge = Color(0xFF53616B),
    dialSurface = Color(0xFF272D32),
    scrim = Color(0x99080C10),
)

val LocalFiendPalette = staticCompositionLocalOf { FiendLightPalette }

object FiendColors {
    val palette: FiendPalette
        @Composable
        @ReadOnlyComposable
        get() = LocalFiendPalette.current

    val canvas: Color @Composable @ReadOnlyComposable get() = palette.canvas
    val surface: Color @Composable @ReadOnlyComposable get() = palette.surface
    val sidebar: Color @Composable @ReadOnlyComposable get() = palette.sidebar
    val panel: Color @Composable @ReadOnlyComposable get() = palette.panel
    val soft: Color @Composable @ReadOnlyComposable get() = palette.soft
    val hover: Color @Composable @ReadOnlyComposable get() = palette.hover
    val line: Color @Composable @ReadOnlyComposable get() = palette.line
    val text: Color @Composable @ReadOnlyComposable get() = palette.text
    val muted: Color @Composable @ReadOnlyComposable get() = palette.muted
    val accent: Color @Composable @ReadOnlyComposable get() = palette.accent
    val accentHover: Color @Composable @ReadOnlyComposable get() = palette.accentHover
    val onAccent: Color @Composable @ReadOnlyComposable get() = palette.onAccent
    val accentSoft: Color @Composable @ReadOnlyComposable get() = palette.accentSoft
    val hero: Color @Composable @ReadOnlyComposable get() = palette.hero
    val green: Color @Composable @ReadOnlyComposable get() = palette.green
    val greenSoft: Color @Composable @ReadOnlyComposable get() = palette.greenSoft
    val danger: Color @Composable @ReadOnlyComposable get() = palette.danger
    val dangerSoft: Color @Composable @ReadOnlyComposable get() = palette.dangerSoft
    val dialEdge: Color @Composable @ReadOnlyComposable get() = palette.dialEdge
    val dialSurface: Color @Composable @ReadOnlyComposable get() = palette.dialSurface
    val scrim: Color @Composable @ReadOnlyComposable get() = palette.scrim
}

internal fun FiendPalette.toColorScheme(dark: Boolean) = if (dark) {
    darkColorScheme(
        primary = accent,
        onPrimary = onAccent,
        primaryContainer = accentSoft,
        onPrimaryContainer = text,
        inversePrimary = accent,
        secondary = muted,
        onSecondary = surface,
        secondaryContainer = soft,
        onSecondaryContainer = text,
        tertiary = green,
        onTertiary = surface,
        tertiaryContainer = greenSoft,
        onTertiaryContainer = green,
        background = canvas,
        onBackground = text,
        surface = surface,
        onSurface = text,
        surfaceVariant = soft,
        onSurfaceVariant = muted,
        surfaceTint = Color.Transparent,
        inverseSurface = text,
        inverseOnSurface = surface,
        error = danger,
        onError = surface,
        errorContainer = dangerSoft,
        onErrorContainer = danger,
        outline = dialEdge,
        outlineVariant = line,
        scrim = scrim,
        surfaceBright = hover,
        surfaceContainer = panel,
        surfaceContainerHigh = soft,
        surfaceContainerHighest = hover,
        surfaceContainerLow = sidebar,
        surfaceContainerLowest = hero,
        surfaceDim = canvas,
        primaryFixed = accentSoft,
        primaryFixedDim = accent,
        onPrimaryFixed = text,
        onPrimaryFixedVariant = accent,
        secondaryFixed = soft,
        secondaryFixedDim = line,
        onSecondaryFixed = text,
        onSecondaryFixedVariant = muted,
        tertiaryFixed = greenSoft,
        tertiaryFixedDim = green,
        onTertiaryFixed = text,
        onTertiaryFixedVariant = green,
    )
} else {
    lightColorScheme(
        primary = accent,
        onPrimary = onAccent,
        primaryContainer = accentSoft,
        onPrimaryContainer = text,
        inversePrimary = accentHover,
        secondary = muted,
        onSecondary = surface,
        secondaryContainer = soft,
        onSecondaryContainer = text,
        tertiary = green,
        onTertiary = surface,
        tertiaryContainer = greenSoft,
        onTertiaryContainer = green,
        background = canvas,
        onBackground = text,
        surface = surface,
        onSurface = text,
        surfaceVariant = soft,
        onSurfaceVariant = muted,
        surfaceTint = Color.Transparent,
        inverseSurface = text,
        inverseOnSurface = surface,
        error = danger,
        onError = onAccent,
        errorContainer = dangerSoft,
        onErrorContainer = danger,
        outline = dialEdge,
        outlineVariant = line,
        scrim = scrim,
        surfaceBright = panel,
        surfaceContainer = panel,
        surfaceContainerHigh = soft,
        surfaceContainerHighest = hover,
        surfaceContainerLow = sidebar,
        surfaceContainerLowest = hero,
        surfaceDim = canvas,
        primaryFixed = accentSoft,
        primaryFixedDim = accent,
        onPrimaryFixed = text,
        onPrimaryFixedVariant = accent,
        secondaryFixed = soft,
        secondaryFixedDim = line,
        onSecondaryFixed = text,
        onSecondaryFixedVariant = muted,
        tertiaryFixed = greenSoft,
        tertiaryFixedDim = green,
        onTertiaryFixed = text,
        onTertiaryFixedVariant = green,
    )
}
