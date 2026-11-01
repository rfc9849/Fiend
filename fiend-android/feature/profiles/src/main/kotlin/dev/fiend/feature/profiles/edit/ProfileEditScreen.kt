package dev.fiend.feature.profiles.edit

import android.content.Context
import android.net.Uri
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExposedDropdownMenu
import androidx.compose.material3.ExposedDropdownMenuAnchorType
import androidx.compose.material3.ExposedDropdownMenuBox
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.fiend.core.designsystem.component.FiendAlertDialog
import dev.fiend.core.designsystem.component.FiendButton
import dev.fiend.core.designsystem.component.FiendButtonKind
import dev.fiend.core.designsystem.component.FiendIconButton
import dev.fiend.core.designsystem.component.FiendRow
import dev.fiend.core.designsystem.component.FiendSectionTitle
import dev.fiend.core.designsystem.component.FiendTextField
import dev.fiend.core.designsystem.component.FiendTopBar
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.core.designsystem.theme.FiendColors
import dev.fiend.core.model.FingerprintTemplate
import dev.fiend.feature.profiles.R
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@Composable
internal fun ProfileEditRoute(viewModel: ProfileEditViewModel, onBack: () -> Unit) {
    val state by viewModel.uiState.collectAsStateWithLifecycle()
    LaunchedEffect(viewModel) { viewModel.saved.collect { onBack() } }
    ProfileEditScreen(
        state = state,
        onChange = viewModel::update,
        onSave = viewModel::save,
        onClose = onBack,
        onSelectTemplate = viewModel::selectTemplate,
        onTemplateFile = viewModel::loadTemplate,
        onTemplateFileFailed = viewModel::templateFileFailed,
    )
}

@Composable
internal fun ProfileEditScreen(
    state: ProfileEditUiState,
    onChange: ((ProfileForm) -> ProfileForm) -> Unit,
    onSave: () -> Unit,
    onClose: () -> Unit,
    onSelectTemplate: (FingerprintTemplate) -> Unit,
    onTemplateFile: (ByteArray) -> Unit,
    onTemplateFileFailed: (String) -> Unit,
) {
    var confirmDiscard by rememberSaveable { mutableStateOf(false) }
    var advancedOpen by rememberSaveable { mutableStateOf(false) }
    var secretVisible by rememberSaveable { mutableStateOf(false) }
    val requestClose = { if (state.dirty) confirmDiscard = true else onClose() }
    BackHandler(state.dirty) { confirmDiscard = true }

    Column(Modifier.fillMaxSize().background(FiendColors.surface)) {
        FiendTopBar(
            title = stringResource(if (state.isNew) R.string.edit_new_title else R.string.edit_title),
            onBack = requestClose,
            backLabel = stringResource(R.string.edit_close),
            navigationIcon = FiendIcons.Close,
            actions = {
                FiendButton(
                    label = stringResource(R.string.edit_save),
                    onClick = onSave,
                    kind = FiendButtonKind.Primary,
                    enabled = !state.saving && state.loaded,
                    busy = state.saving,
                    compact = true,
                )
            },
        )
        if (state.loaded) {
            val f = state.form
            Column(
                modifier = Modifier
                    .weight(1f)
                    .fillMaxWidth()
                    .verticalScroll(rememberScrollState())
                    .imePadding()
                    .padding(horizontal = 16.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
            Column(
                modifier = Modifier
                    .widthIn(max = 720.dp)
                    .padding(bottom = 32.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                state.saveError?.let {
                    Text(it, color = FiendColors.danger, style = MaterialTheme.typography.bodyMedium)
                }
                FiendSectionTitle(stringResource(R.string.edit_group_general))
                FormField(
                    value = f.name,
                    onValueChange = { v -> onChange { it.copy(name = v) } },
                    label = stringResource(R.string.edit_name),
                    icon = FiendIcons.Flag,
                    error = state.errors[Field.Name],
                    capitalization = KeyboardCapitalization.Words,
                )
                FiendSectionTitle(stringResource(R.string.edit_group_server))
                FormField(
                    value = f.host,
                    onValueChange = { v -> onChange { it.copy(host = v) } },
                    label = stringResource(R.string.edit_host),
                    icon = FiendIcons.Public,
                    error = state.errors[Field.Host],
                    keyboardType = KeyboardType.Uri,
                )
                FormField(
                    value = f.port,
                    onValueChange = { v -> onChange { it.copy(port = v.filter(Char::isDigit).take(5)) } },
                    label = stringResource(R.string.edit_port),
                    icon = FiendIcons.Hub,
                    error = state.errors[Field.Port],
                    keyboardType = KeyboardType.Number,
                )
                FormField(
                    value = f.moreServers,
                    onValueChange = { v -> onChange { it.copy(moreServers = v) } },
                    label = stringResource(R.string.edit_more_servers),
                    icon = FiendIcons.Public,
                    error = state.errors[Field.MoreServers],
                    supporting = stringResource(R.string.edit_more_servers_hint),
                    keyboardType = KeyboardType.Uri,
                )
                FormField(
                    value = f.secret,
                    onValueChange = { v -> onChange { it.copy(secret = v.trim().take(64)) } },
                    label = stringResource(R.string.edit_secret),
                    icon = FiendIcons.Key,
                    error = state.errors[Field.Secret],
                    supporting = "${stringResource(R.string.edit_secret_hint)} · ${f.secret.length}/64",
                    keyboardType = KeyboardType.Password,
                    visualTransformation = if (secretVisible) VisualTransformation.None else PasswordVisualTransformation(),
                    trailing = {
                        FiendIconButton(
                            icon = if (secretVisible) FiendIcons.VisibilityOff else FiendIcons.Visibility,
                            label = stringResource(if (secretVisible) R.string.detail_hide_secret else R.string.detail_show_secret),
                            onClick = { secretVisible = !secretVisible },
                        )
                    },
                )
                FiendSectionTitle(stringResource(R.string.edit_group_fingerprint))
                TemplatePicker(state, onSelectTemplate, onTemplateFile, onTemplateFileFailed)

                FiendRow(
                    headline = stringResource(R.string.edit_group_advanced),
                    icon = FiendIcons.Tune,
                    trailing = {
                        val rotation by animateFloatAsState(if (advancedOpen) 180f else 0f, label = "chevron")
                        Icon(
                            FiendIcons.ExpandMore,
                            contentDescription = null,
                            tint = FiendColors.muted,
                            modifier = Modifier.rotate(rotation),
                        )
                    },
                    onClick = { advancedOpen = !advancedOpen },
                    modifier = Modifier.padding(top = 8.dp),
                )
                AnimatedVisibility(visible = advancedOpen) {
                    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            NumberField(f.handshake, { v -> onChange { it.copy(handshake = v) } }, R.string.edit_handshake, state.errors[Field.Handshake], Modifier.weight(1f))
                            NumberField(f.connections, { v -> onChange { it.copy(connections = v) } }, R.string.edit_connections, state.errors[Field.Connections], Modifier.weight(1f))
                        }
                        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            NumberField(f.udpIdle, { v -> onChange { it.copy(udpIdle = v) } }, R.string.edit_udp_idle, state.errors[Field.UdpIdle], Modifier.weight(1f))
                            Spacer(Modifier.weight(1f))
                        }
                    }
                }
                Spacer(Modifier.height(16.dp))
            }
        }
        }
    }

    if (confirmDiscard) {
        FiendAlertDialog(
            title = stringResource(R.string.edit_discard_title),
            confirmLabel = stringResource(R.string.edit_discard),
            onConfirm = {
                confirmDiscard = false
                onClose()
            },
            dismissLabel = stringResource(R.string.edit_keep),
            onDismiss = { confirmDiscard = false },
        ) {
            Text(
                stringResource(R.string.edit_discard_body),
                style = MaterialTheme.typography.bodyMedium,
                color = FiendColors.muted,
            )
        }
    }
}

@Composable
private fun errorText(error: FieldError?): String? = error?.let {
    stringResource(
        when (it) {
            FieldError.Required -> R.string.edit_error_required
            FieldError.Port -> R.string.edit_error_port
            FieldError.Servers -> R.string.edit_error_servers
            FieldError.Secret -> R.string.edit_error_secret
            FieldError.Number -> R.string.edit_error_number
        },
    )
}

@Composable
private fun FormField(
    value: String,
    onValueChange: (String) -> Unit,
    label: String,
    icon: ImageVector,
    error: FieldError?,
    supporting: String? = null,
    keyboardType: KeyboardType = KeyboardType.Text,
    capitalization: KeyboardCapitalization = KeyboardCapitalization.None,
    visualTransformation: VisualTransformation = VisualTransformation.None,
    trailing: @Composable (() -> Unit)? = null,
) {
    FiendTextField(
        value = value,
        onValueChange = onValueChange,
        label = label,
        leadingIcon = icon,
        trailing = trailing,
        supporting = errorText(error) ?: supporting,
        isError = error != null,
        singleLine = true,
        visualTransformation = visualTransformation,
        keyboardOptions = KeyboardOptions(keyboardType = keyboardType, capitalization = capitalization),
    )
}

@Composable
private fun NumberField(value: String, onValueChange: (String) -> Unit, label: Int, error: FieldError?, modifier: Modifier) {
    FiendTextField(
        value = value,
        onValueChange = { onValueChange(it.filter(Char::isDigit).take(7)) },
        label = stringResource(label),
        supporting = errorText(error),
        isError = error != null,
        singleLine = true,
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
        modifier = modifier,
    )
}

@Composable
private fun TemplatePicker(
    state: ProfileEditUiState,
    onSelect: (FingerprintTemplate) -> Unit,
    onFile: (ByteArray) -> Unit,
    onFileFailed: (String) -> Unit,
) {
    var expanded by rememberSaveable { mutableStateOf(false) }
    val selected = state.templates.firstOrNull { it.id == state.form.templateId }
    val context = LocalContext.current
    val resources = LocalResources.current
    val scope = rememberCoroutineScope()
    val openFile = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri != null) scope.launch {
            runCatching { withContext(Dispatchers.IO) { readTemplateFile(context, uri) } }
                .onSuccess(onFile)
                .onFailure { onFileFailed(resources.getString(R.string.edit_template_read_failed)) }
        }
    }
    ExposedDropdownMenuBox(expanded = expanded, onExpandedChange = { expanded = it }) {
        FiendTextField(
            value = selected?.label.orEmpty(),
            onValueChange = {},
            readOnly = true,
            label = stringResource(R.string.edit_template),
            leadingIcon = FiendIcons.Fingerprint,
            trailing = {
                Icon(
                    FiendIcons.ExpandMore,
                    contentDescription = null,
                    tint = FiendColors.muted,
                    modifier = Modifier.rotate(if (expanded) 180f else 0f),
                )
            },
            isError = state.templateError != null,
            supporting = state.templateError ?: stringResource(R.string.edit_template_hint),
            singleLine = true,
            modifier = Modifier.menuAnchor(ExposedDropdownMenuAnchorType.PrimaryNotEditable),
        )
        ExposedDropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
            state.templates.forEach { template ->
                DropdownMenuItem(
                    text = { Text(template.label) },
                    leadingIcon = if (template.id == state.form.templateId) {
                        { Icon(FiendIcons.Check, contentDescription = null) }
                    } else {
                        null
                    },
                    onClick = {
                        expanded = false
                        onSelect(template)
                    },
                )
            }
            DropdownMenuItem(
                text = { Text(stringResource(R.string.edit_template_file)) },
                leadingIcon = { Icon(FiendIcons.UploadFile, contentDescription = null) },
                onClick = {
                    expanded = false
                    openFile.launch(arrayOf("*/*"))
                },
            )
        }
    }
}

private fun readTemplateFile(context: Context, uri: Uri): ByteArray = context.contentResolver.openInputStream(uri)?.use { input ->
    val bytes = ByteArray(MAX_TEMPLATE_FILE + 1)
    var used = 0
    while (used < bytes.size) {
        val size = input.read(bytes, used, bytes.size - used)
        if (size < 0) break
        used += size
    }
    require(used <= MAX_TEMPLATE_FILE) { "Template file is too large" }
    bytes.copyOf(used)
} ?: error("Cannot open the template file")

private const val MAX_TEMPLATE_FILE = 131072
