package dev.fiend.feature.settings

import android.content.ClipData
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.ClipEntry
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.fiend.core.designsystem.icon.FiendIcons
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

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.logs_title)) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(FiendIcons.ArrowBack, contentDescription = stringResource(R.string.logs_back))
                    }
                },
                actions = {
                    IconButton(onClick = {
                        val text = lines.joinToString("\n") { "${formatter.format(Date(it.timeEpochMs))} ${it.level.name.uppercase()} ${it.target}: ${it.message}" }
                        scope.launch {
                            clipboard.setClipEntry(ClipEntry(ClipData.newPlainText("fiend logs", text)))
                            snackbar.showSnackbar(resources.getString(R.string.logs_copied))
                        }
                    }) {
                        Icon(FiendIcons.ContentCopy, contentDescription = stringResource(R.string.logs_copy))
                    }
                    IconButton(onClick = onClear) {
                        Icon(FiendIcons.Delete, contentDescription = stringResource(R.string.logs_clear))
                    }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        Surface(
            color = MaterialTheme.colorScheme.surfaceContainer,
            shape = MaterialTheme.shapes.extraLarge,
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .padding(start = 16.dp, end = 16.dp, bottom = 16.dp),
        ) {
            if (lines.isEmpty()) {
                Box(contentAlignment = Alignment.Center) {
                    Text(
                        stringResource(R.string.logs_empty),
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        style = MaterialTheme.typography.bodyLarge,
                    )
                }
            } else {
                LazyColumn(
                    state = listState,
                    contentPadding = PaddingValues(16.dp),
                    verticalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    items(lines) { line -> LogRow(line, formatter.format(Date(line.timeEpochMs))) }
                }
            }
        }
    }
}

@Composable
private fun LogRow(line: LogLine, time: String) {
    val c = MaterialTheme.colorScheme
    val (bg, fg) = when (line.level) {
        LogLevel.Debug -> c.surfaceContainerHighest to c.onSurfaceVariant
        LogLevel.Info -> c.primaryContainer to c.onPrimaryContainer
        LogLevel.Warn -> c.tertiaryContainer to c.onTertiaryContainer
        LogLevel.Error -> c.errorContainer to c.onErrorContainer
    }
    Row(verticalAlignment = Alignment.Top, modifier = Modifier.fillMaxWidth()) {
        Surface(color = bg, contentColor = fg, shape = MaterialTheme.shapes.small, modifier = Modifier.width(52.dp)) {
            Text(
                text = line.level.name.uppercase(),
                style = MaterialTheme.typography.labelSmall,
                fontFamily = FontFamily.Monospace,
                modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
                maxLines = 1,
            )
        }
        Box(Modifier.padding(start = 10.dp)) {
            Text(
                text = "$time  ${line.target}\n${line.message}",
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
                color = if (line.level == LogLevel.Debug) c.onSurfaceVariant else Color.Unspecified,
            )
        }
    }
}
