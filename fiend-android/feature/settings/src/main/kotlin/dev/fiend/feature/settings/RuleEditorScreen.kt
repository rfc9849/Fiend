package dev.fiend.feature.settings

import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.core.model.RuleKind

@Composable
internal fun RuleEditorScreen(
    kind: RuleKind,
    state: RuleEditorUiState,
    onText: (String) -> Unit,
    onImport: (String) -> Unit,
    onImportShown: () -> Unit,
    onSave: () -> Unit,
    onClose: () -> Unit,
) {
    var confirmDiscard by rememberSaveable { mutableStateOf(false) }
    val snackbar = remember { SnackbarHostState() }
    val importedText = stringResource(R.string.rules_imported)
    val requestClose = { if (state.dirty) confirmDiscard = true else onClose() }
    BackHandler(state.dirty) { confirmDiscard = true }
    LaunchedEffect(state.imported) {
        if (state.imported) {
            snackbar.showSnackbar(importedText)
            onImportShown()
        }
    }
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri != null) onImport(uri.toString())
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(kind.title())) },
                navigationIcon = {
                    IconButton(onClick = requestClose) {
                        Icon(FiendIcons.Close, contentDescription = stringResource(R.string.rules_close))
                    }
                },
                actions = {
                    IconButton(onClick = { picker.launch(arrayOf("*/*")) }, enabled = state.loaded) {
                        Icon(FiendIcons.UploadFile, contentDescription = stringResource(R.string.rules_import))
                    }
                    Button(
                        onClick = onSave,
                        enabled = state.loaded && !state.saving,
                        modifier = Modifier.padding(end = 12.dp),
                    ) { Text(stringResource(R.string.routing_save)) }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        if (!state.loaded) return@Scaffold
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .imePadding()
                .padding(horizontal = 16.dp),
        ) {
            OutlinedTextField(
                value = state.text,
                onValueChange = onText,
                placeholder = { Text(stringResource(kind.example())) },
                supportingText = {
                    Text(
                        state.error
                            ?: stringResource(R.string.rules_counts, state.counts.total, state.counts.domains + state.counts.exact + state.counts.keywords, state.counts.cidrs),
                    )
                },
                isError = state.error != null,
                textStyle = MaterialTheme.typography.bodyMedium.copy(fontFamily = FontFamily.Monospace),
                keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false),
                shape = MaterialTheme.shapes.large,
                modifier = Modifier
                    .fillMaxWidth()
                    .weight(1f),
            )
            Text(
                text = stringResource(kind.syntax()),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 8.dp, vertical = 12.dp),
            )
        }
    }

    if (confirmDiscard) {
        AlertDialog(
            onDismissRequest = { confirmDiscard = false },
            title = { Text(stringResource(R.string.rules_discard_title)) },
            text = { Text(stringResource(R.string.rules_discard_body)) },
            confirmButton = {
                TextButton(onClick = {
                    confirmDiscard = false
                    onClose()
                }) { Text(stringResource(R.string.rules_discard)) }
            },
            dismissButton = {
                TextButton(onClick = { confirmDiscard = false }) { Text(stringResource(R.string.rules_keep)) }
            },
        )
    }
}

private fun RuleKind.title() = when (this) {
    RuleKind.Direct -> R.string.routing_direct_rules
    RuleKind.Block -> R.string.routing_block_rules
    RuleKind.Proxy -> R.string.routing_proxy_rules
    RuleKind.Dpi -> R.string.routing_dpi_rules
}

private fun RuleKind.example() = when (this) {
    RuleKind.Direct -> R.string.rules_example_direct
    RuleKind.Block -> R.string.rules_example_block
    RuleKind.Proxy -> R.string.rules_example_proxy
    RuleKind.Dpi -> R.string.rules_example_dpi
}

private fun RuleKind.syntax() = when (this) {
    RuleKind.Direct -> R.string.rules_syntax_direct
    RuleKind.Block -> R.string.rules_syntax_block
    RuleKind.Proxy -> R.string.rules_syntax_proxy
    RuleKind.Dpi -> R.string.rules_syntax_dpi
}
