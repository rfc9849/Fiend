package dev.fiend.feature.profiles.detail

import android.content.ClipData
import android.content.Intent
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Button
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.SheetValue
import androidx.compose.material3.rememberBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.ClipEntry
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.feature.profiles.R
import kotlinx.coroutines.launch

@Composable
internal fun ShareSheet(name: String, link: String, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val clipboard = LocalClipboard.current
    val scope = rememberCoroutineScope()
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberBottomSheetState(initialValue = SheetValue.Hidden, enabledValues = setOf(SheetValue.Hidden, SheetValue.Expanded)),
        containerColor = MaterialTheme.colorScheme.surfaceContainerLow,
    ) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            modifier = Modifier
                .fillMaxWidth()
                .navigationBarsPadding()
                .padding(horizontal = 24.dp),
        ) {
            Text(stringResource(R.string.share_title), style = MaterialTheme.typography.headlineSmallEmphasized)
            Spacer(Modifier.height(4.dp))
            Text(name, style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Spacer(Modifier.height(24.dp))
            Surface(color = Color.White, shape = MaterialTheme.shapes.extraLarge) {
                val description = stringResource(R.string.share_qr, name)
                QrCode(
                    payload = link,
                    modifier = Modifier
                        .padding(20.dp)
                        .size(208.dp)
                        .semantics { contentDescription = description },
                )
            }
            Spacer(Modifier.height(20.dp))
            Surface(
                color = MaterialTheme.colorScheme.surfaceContainerHighest,
                shape = MaterialTheme.shapes.large,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text(
                    text = link,
                    style = MaterialTheme.typography.bodyMedium,
                    fontFamily = MonoFamily,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.padding(16.dp),
                )
            }
            Spacer(Modifier.height(12.dp))
            Text(
                stringResource(R.string.share_body),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(24.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp), modifier = Modifier.fillMaxWidth()) {
                FilledTonalButton(
                    onClick = {
                        scope.launch { clipboard.setClipEntry(ClipEntry(ClipData.newPlainText(name, link))) }
                    },
                    modifier = Modifier
                        .weight(1f)
                        .height(56.dp),
                ) {
                    Icon(FiendIcons.ContentCopy, contentDescription = null)
                    Spacer(Modifier.width(8.dp))
                    Text(stringResource(R.string.share_copy))
                }
                Button(
                    onClick = {
                        val send = Intent(Intent.ACTION_SEND).apply {
                            type = "text/plain"
                            putExtra(Intent.EXTRA_TEXT, link)
                            putExtra(Intent.EXTRA_TITLE, name)
                        }
                        context.startActivity(Intent.createChooser(send, null))
                    },
                    modifier = Modifier
                        .weight(1f)
                        .height(56.dp),
                ) {
                    Icon(FiendIcons.Share, contentDescription = null)
                    Spacer(Modifier.width(8.dp))
                    Text(stringResource(R.string.share_send))
                }
            }
            Spacer(Modifier.height(16.dp))
        }
    }
}

@Composable
private fun QrCode(payload: String, modifier: Modifier = Modifier) {
    val matrix = remember(payload) { qrMatrix(payload) }
    Box(modifier, contentAlignment = Alignment.Center) {
        if (matrix == null) {
            Text(stringResource(R.string.share_qr_too_large), color = Color(0xFF1B1B1F), textAlign = TextAlign.Center)
        } else {
            Canvas(Modifier.matchParentSize()) {
                val cell = size.minDimension / matrix.width
                for (y in 0 until matrix.height) for (x in 0 until matrix.width) {
                    if (matrix[x, y]) drawRect(Color(0xFF1B1B1F), Offset(x * cell, y * cell), Size(cell, cell))
                }
            }
        }
    }
}
