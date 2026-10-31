package dev.fiend.feature.home

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.core.designsystem.theme.FiendColors

enum class ConnectPhase { Idle, Busy, Active, Error }

@Composable
fun ConnectButton(
    phase: ConnectPhase,
    onClick: () -> Unit,
    contentDescription: String,
    stateDescription: String,
    modifier: Modifier = Modifier,
    size: Dp = 160.dp,
) {
    val ring = when (phase) {
        ConnectPhase.Active -> FiendColors.green
        ConnectPhase.Error -> FiendColors.danger
        else -> FiendColors.dialEdge
    }
    val glyph = when (phase) {
        ConnectPhase.Active -> FiendColors.green
        ConnectPhase.Error -> FiendColors.danger
        else -> FiendColors.muted
    }
    val dialSurface = FiendColors.dialSurface
    val line = FiendColors.line
    val accent = FiendColors.accent
    val rotation = remember { Animatable(0f) }
    LaunchedEffect(phase) {
        if (phase == ConnectPhase.Busy) {
            rotation.snapTo(0f)
            while (true) {
                rotation.animateTo(rotation.value + 360f, tween(1_200, easing = LinearEasing))
            }
        }
    }

    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    val scale by animateFloatAsState(if (pressed) 0.985f else 1f, label = "powerPress")
    val haptics = LocalHapticFeedback.current

    Box(
        modifier = modifier
            .size(size)
            .graphicsLayer {
                scaleX = scale
                scaleY = scale
            }
            .clickable(
                interactionSource = interaction,
                indication = null,
                role = Role.Button,
            ) {
                haptics.performHapticFeedback(
                    if (phase == ConnectPhase.Idle || phase == ConnectPhase.Error) {
                        HapticFeedbackType.ToggleOn
                    } else {
                        HapticFeedbackType.ToggleOff
                    },
                )
                onClick()
            }
            .semantics {
                this.contentDescription = contentDescription
                this.stateDescription = stateDescription
            },
        contentAlignment = Alignment.Center,
    ) {
        Canvas(Modifier.matchParentSize()) {
            val side = this.size.minDimension
            val hairline = 1.dp.toPx()
            val outer = side / 2 - 9.dp.toPx()
            val inner = side / 2 - 17.dp.toPx()
            val center = Offset(this.size.width / 2, this.size.height / 2)
            drawCircle(dialSurface, radius = inner, center = center)
            drawCircle(line, radius = inner, center = center, style = Stroke(hairline))
            drawCircle(ring, radius = outer, center = center, style = Stroke(if (phase == ConnectPhase.Busy) 2.dp.toPx() else hairline))
            if (phase == ConnectPhase.Busy) {
                drawArc(
                    color = accent,
                    startAngle = rotation.value - 90f,
                    sweepAngle = 80f,
                    useCenter = false,
                    topLeft = Offset(center.x - outer, center.y - outer),
                    size = Size(outer * 2, outer * 2),
                    style = Stroke(2.dp.toPx(), cap = StrokeCap.Round),
                )
            }
        }
        Icon(
            imageVector = FiendIcons.PowerSettingsNew,
            contentDescription = null,
            tint = glyph,
            modifier = Modifier.size(size * 0.29f),
        )
    }
}
