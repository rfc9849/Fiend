package dev.fiend.core.designsystem.component

import java.util.Locale
import kotlin.math.ln
import kotlin.math.pow

data class Measure(val value: String, val unit: String)

fun formatBitrate(bitsPerSecond: Long): Measure {
    if (bitsPerSecond <= 0) return Measure("0", "Kbps")
    val units = listOf("bps", "Kbps", "Mbps", "Gbps")
    val exp = (ln(bitsPerSecond.toDouble()) / ln(1000.0)).toInt().coerceIn(0, units.lastIndex)
    val scaled = bitsPerSecond / 1000.0.pow(exp)
    return Measure(formatScaled(scaled), units[exp])
}

fun formatBytes(bytes: Long): Measure {
    if (bytes <= 0) return Measure("0", "MB")
    val units = listOf("B", "KB", "MB", "GB", "TB")
    val exp = (ln(bytes.toDouble()) / ln(1024.0)).toInt().coerceIn(0, units.lastIndex)
    val scaled = bytes / 1024.0.pow(exp)
    return Measure(formatScaled(scaled), units[exp])
}

fun formatElapsed(millis: Long): String {
    val total = (millis / 1000).coerceAtLeast(0)
    val h = total / 3600
    val m = (total % 3600) / 60
    val s = total % 60
    return if (h > 0) String.format(Locale.ROOT, "%d:%02d:%02d", h, m, s) else String.format(Locale.ROOT, "%02d:%02d", m, s)
}

private fun formatScaled(value: Double): String = when {
    value >= 100 -> String.format(Locale.getDefault(), "%.0f", value)
    value >= 10 -> String.format(Locale.getDefault(), "%.1f", value)
    else -> String.format(Locale.getDefault(), "%.2f", value)
}
