package dev.fiend.feature.home

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.toPath
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.center
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Matrix
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.graphics.shapes.Morph
import androidx.graphics.shapes.RoundedPolygon
import dev.fiend.core.designsystem.icon.FiendIcons

enum class ConnectPhase { Idle, Busy, Active, Error }

@Composable
fun ConnectButton(
    phase: ConnectPhase,
    onClick: () -> Unit,
    contentDescription: String,
    stateDescription: String,
    modifier: Modifier = Modifier,
    size: Dp = 208.dp,
) {
    val colors = MaterialTheme.colorScheme
    val motion = MaterialTheme.motionScheme
    val target: RoundedPolygon = when (phase) {
        ConnectPhase.Idle -> MaterialShapes.Cookie9Sided
        ConnectPhase.Busy -> MaterialShapes.SoftBurst
        ConnectPhase.Active -> MaterialShapes.Cookie12Sided
        ConnectPhase.Error -> MaterialShapes.Cookie6Sided
    }

    var from by remember { mutableStateOf(target) }
    var to by remember { mutableStateOf(target) }
    val progress = remember { Animatable(1f) }
    val morphSpec = motion.slowSpatialSpec<Float>()
    LaunchedEffect(target) {
        if (target != to) {
            from = to
            to = target
            progress.snapTo(0f)
            progress.animateTo(1f, morphSpec)
        }
    }
    val morph = remember(from, to) { Morph(from, to) }

    val rotation = remember { Animatable(0f) }
    val settleSpec = motion.slowSpatialSpec<Float>()
    LaunchedEffect(phase) {
        when (phase) {
            ConnectPhase.Busy -> while (true) {
                rotation.animateTo(rotation.value + 360f, tween(1_400, easing = LinearEasing))
            }
            ConnectPhase.Active -> while (true) {
                rotation.animateTo(rotation.value + 360f, tween(40_000, easing = LinearEasing))
            }
            ConnectPhase.Idle, ConnectPhase.Error -> {
                val step = 40f
                rotation.animateTo((rotation.value / step).let { kotlin.math.round(it) } * step, settleSpec)
            }
        }
    }

    val container by animateColorAsState(
        targetValue = when (phase) {
            ConnectPhase.Idle -> colors.secondaryContainer
            ConnectPhase.Busy -> colors.primaryContainer
            ConnectPhase.Active -> colors.primary
            ConnectPhase.Error -> colors.error
        },
        animationSpec = motion.slowEffectsSpec(),
        label = "container",
    )
    val content by animateColorAsState(
        targetValue = when (phase) {
            ConnectPhase.Idle -> colors.onSecondaryContainer
            ConnectPhase.Busy -> colors.onPrimaryContainer
            ConnectPhase.Active -> colors.onPrimary
            ConnectPhase.Error -> colors.onError
        },
        animationSpec = motion.slowEffectsSpec(),
        label = "content",
    )

    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    val scale by animateFloatAsState(
        targetValue = if (pressed) 0.9f else 1f,
        animationSpec = motion.fastSpatialSpec(),
        label = "press",
    )
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
        MorphCanvas(morph, { progress.value }, { rotation.value }, container)
        Icon(
            imageVector = FiendIcons.PowerSettingsNew,
            contentDescription = null,
            tint = content,
            modifier = Modifier.size(size * 0.3f),
        )
    }
}

@Composable
private fun BoxScope.MorphCanvas(
    morph: Morph,
    progress: () -> Float,
    rotation: () -> Float,
    color: Color,
) {
    val path = remember { Path() }
    val matrix = remember { Matrix() }
    Canvas(
        modifier = Modifier
            .matchParentSize()
            .graphicsLayer { rotationZ = rotation() },
    ) {
        path.rewind()
        morph.toPath(progress(), path)
        matrix.reset()
        matrix.scale(size.width, size.height)
        path.transform(matrix)
        path.translate(size.center - path.getBounds().center)
        drawPath(path, color)
    }
}
