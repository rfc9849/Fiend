package dev.fiend.feature.settings

import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.fiend.core.designsystem.component.FiendAlertDialog
import dev.fiend.core.designsystem.component.FiendButton
import dev.fiend.core.designsystem.component.FiendButtonKind
import dev.fiend.core.designsystem.component.FiendIconButton
import dev.fiend.core.designsystem.component.FiendTextField
import dev.fiend.core.designsystem.component.FiendTopBar
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.core.designsystem.theme.FiendColors
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

    Box(Modifier.fillMaxSize().background(FiendColors.surface)) {
        Column(Modifier.fillMaxSize()) {
            FiendTopBar(
                title = stringResource(kind.title()),
                onBack = requestClose,
                backLabel = stringResource(R.string.rules_close),
                navigationIcon = FiendIcons.Close,
                actions = {
                    FiendIconButton(
                        icon = FiendIcons.UploadFile,
                        label = stringResource(R.string.rules_import),
                        onClick = { picker.launch(arrayOf("*/*")) },
                        enabled = state.loaded,
                    )
                    FiendButton(
                        label = stringResource(R.string.routing_save),
                        onClick = onSave,
                        kind = FiendButtonKind.Primary,
                        enabled = state.loaded && !state.saving,
                        busy = state.saving,
                        compact = true,
                    )
                },
            )
            Column(
                modifier = Modifier
                    .weight(1f)
                    .fillMaxWidth()
                    .imePadding()
                    .padding(horizontal = 16.dp, vertical = 8.dp),
            ) {
                if (state.loaded) {
                    FiendTextField(
                        value = state.text,
                        onValueChange = onText,
                        placeholder = stringResource(kind.example()),
                        supporting = state.error
                            ?: stringResource(R.string.rules_counts, state.counts.total, state.counts.domains + state.counts.exact + state.counts.keywords, state.counts.cidrs),
                        isError = state.error != null,
                        singleLine = false,
                        textStyle = MaterialTheme.typography.bodySmall.copy(
                            fontFamily = FontFamily.Monospace,
                            fontSize = 12.sp,
                            lineHeight = 22.sp,
                        ),
                        keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false),
                        modifier = Modifier
                            .fillMaxWidth()
                            .weight(1f),
                    )
                    Spacer(Modifier.height(10.dp))
                    Text(
                        text = stringResource(kind.syntax()),
                        style = MaterialTheme.typography.bodySmall,
                        color = FiendColors.muted,
                        modifier = Modifier.padding(horizontal = 4.dp, vertical = 8.dp),
                    )
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

    if (confirmDiscard) {
        FiendAlertDialog(
            title = stringResource(R.string.rules_discard_title),
            confirmLabel = stringResource(R.string.rules_discard),
            onConfirm = {
                confirmDiscard = false
                onClose()
            },
            dismissLabel = stringResource(R.string.rules_keep),
            onDismiss = { confirmDiscard = false },
        ) {
            Text(
                stringResource(R.string.rules_discard_body),
                style = MaterialTheme.typography.bodyMedium,
                color = FiendColors.muted,
            )
        }
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
