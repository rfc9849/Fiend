package dev.fiend.core.designsystem.component

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.fiend.core.designsystem.theme.FiendColors

fun avatarLabel(name: String, code: String?): String =
    code?.take(2)?.uppercase()
        ?: name.split(' ', '-', '_').filter { it.isNotBlank() }.take(2).joinToString("") { it.take(1) }.uppercase()
            .ifEmpty { "?" }

@Composable
fun ShapeAvatar(
    label: String,
    seed: String,
    modifier: Modifier = Modifier,
    size: Dp = 44.dp,
) {
    val shape = RoundedCornerShape(4.dp)
    Box(
        modifier = modifier
            .size(size)
            .background(FiendColors.soft, shape)
            .border(1.dp, FiendColors.line, shape),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = label,
            color = FiendColors.muted,
            fontSize = (size.value * 0.25f).coerceIn(10f, 16f).sp,
            lineHeight = (size.value * 0.25f).coerceIn(10f, 16f).sp,
            fontWeight = FontWeight.Medium,
            letterSpacing = 0.3.sp,
            maxLines = 1,
            style = MaterialTheme.typography.labelMedium,
        )
    }
}
