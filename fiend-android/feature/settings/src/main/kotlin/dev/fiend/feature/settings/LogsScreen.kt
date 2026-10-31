package dev.fiend.feature.settings

import android.content.ClipData
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.ClipEntry
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.fiend.core.designsystem.component.FiendIconButton
import dev.fiend.core.designsystem.component.FiendTopBar
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.core.designsystem.theme.FiendColors
import dev.fiend.core.model.LogLevel
import dev.fiend.core.model.LogLine
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlinx.coroutines.launch

@Composable
internal fun LogsScreen(lines: List<LogLine>, onBack: () -> Unit, onClear: () -> Unit) {
    val clipboard = LocalClipboard.current
    val resources = LocalResources.current
    val snackbar = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()
    val listState = rememberLazyListState()
    val formatter = remember { SimpleDateFormat("HH:mm:ss.SSS", Locale.ROOT) }

    LaunchedEffect(lines.size) {
        if (lines.isNotEmpty()) listState.animateScrollToItem(lines.lastIndex)
    }

    Box(Modifier.fillMaxSize().background(FiendColors.surface)) {
        Column(Modifier.fillMaxSize()) {
            FiendTopBar(
                title = stringResource(R.string.logs_title),
                onBack = onBack,
                backLabel = stringResource(R.string.settings_back),
                actions = {
                    FiendIconButton(
                        icon = FiendIcons.ContentCopy,
                        label = stringResource(R.string.logs_copy),
                        onClick = {
                            val text = lines.joinToString("\n") {
                                "${formatter.format(Date(it.timeEpochMs))} ${it.level.name.uppercase()} ${it.target}: ${it.message}"
                            }
                            scope.launch {
                                clipboard.setClipEntry(ClipEntry(ClipData.newPlainText("fiend logs", text)))
                                snackbar.showSnackbar(resources.getString(R.string.logs_copied))
                            }
                        },
                    )
                    FiendIconButton(
                        icon = FiendIcons.Delete,
                        label = stringResource(R.string.logs_clear),
                        onClick = onClear,
                    )
                },
            )
            Surface(
                color = FiendColors.panel,
                shape = RoundedCornerShape(6.dp),
                border = BorderStroke(1.dp, FiendColors.line),
                modifier = Modifier
                    .weight(1f)
                    .fillMaxWidth()
                    .padding(start = 16.dp, end = 16.dp, bottom = 16.dp),
            ) {
                if (lines.isEmpty()) {
                    Box(contentAlignment = Alignment.Center, modifier = Modifier.fillMaxSize()) {
                        Text(
                            stringResource(R.string.logs_empty),
                            color = FiendColors.muted,
                            style = MaterialTheme.typography.bodyMedium,
                        )
                    }
                } else {
                    LazyColumn(
                        state = listState,
                        contentPadding = PaddingValues(vertical = 4.dp),
                    ) {
                        itemsIndexed(lines) { index, line ->
                            if (index > 0) HorizontalDivider(color = FiendColors.line, thickness = 1.dp)
                            LogRow(line, formatter.format(Date(line.timeEpochMs)))
                        }
                    }
                }
            }
        }
        SnackbarHost(
            hostState = snackbar,
            modifier = Modifier
                .align(Alignment.BottomCenter)
                .padding(bottom = 16.dp),
        )
    }
}

@Composable
private fun LogRow(line: LogLine, time: String) {
    val levelColor = when (line.level) {
        LogLevel.Debug -> FiendColors.muted
        LogLevel.Info -> FiendColors.accent
        LogLevel.Warn, LogLevel.Error -> FiendColors.danger
    }
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 14.dp, vertical = 10.dp),
        verticalAlignment = Alignment.Top,
    ) {
        Text(
            text = time,
            style = MaterialTheme.typography.labelSmall.copy(fontFamily = FontFamily.Monospace, fontSize = 11.sp),
            color = FiendColors.muted,
            modifier = Modifier.width(84.dp),
        )
        Text(
            text = line.level.name.uppercase(),
            style = MaterialTheme.typography.labelSmall.copy(fontFamily = FontFamily.Monospace, fontSize = 11.sp),
            color = levelColor,
            modifier = Modifier.width(52.dp),
        )
        Column(Modifier.weight(1f)) {
            Text(
                text = line.target,
                style = MaterialTheme.typography.labelSmall.copy(fontFamily = FontFamily.Monospace, fontSize = 12.sp),
                color = FiendColors.muted,
            )
            Text(
                text = line.message,
                style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace, fontSize = 13.sp),
                color = FiendColors.text,
            )
        }
    }
}
