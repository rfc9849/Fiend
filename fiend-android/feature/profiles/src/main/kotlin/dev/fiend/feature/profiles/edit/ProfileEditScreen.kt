package dev.fiend.feature.profiles.edit

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.animateFloatAsState
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
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExposedDropdownMenuAnchorType
import androidx.compose.material3.ExposedDropdownMenu
import androidx.compose.material3.ExposedDropdownMenuBox
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LargeFlexibleTopAppBar
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.fiend.core.designsystem.component.FiendListDefaults
import dev.fiend.core.designsystem.component.GroupHeader
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.feature.profiles.R

@Composable
internal fun ProfileEditRoute(viewModel: ProfileEditViewModel, onBack: () -> Unit) {
    val state by viewModel.uiState.collectAsStateWithLifecycle()
    LaunchedEffect(viewModel) { viewModel.saved.collect { onBack() } }
    ProfileEditScreen(state = state, onChange = viewModel::update, onSave = viewModel::save, onClose = onBack)
}

@Composable
internal fun ProfileEditScreen(
    state: ProfileEditUiState,
    onChange: ((ProfileForm) -> ProfileForm) -> Unit,
    onSave: () -> Unit,
    onClose: () -> Unit,
) {
    val scrollBehavior = TopAppBarDefaults.exitUntilCollapsedScrollBehavior()
    var confirmDiscard by rememberSaveable { mutableStateOf(false) }
    var advancedOpen by rememberSaveable { mutableStateOf(false) }
    var secretVisible by rememberSaveable { mutableStateOf(false) }
    val requestClose = { if (state.dirty) confirmDiscard = true else onClose() }
    BackHandler(state.dirty) { confirmDiscard = true }

    Scaffold(
        modifier = Modifier.nestedScroll(scrollBehavior.nestedScrollConnection),
        topBar = {
            LargeFlexibleTopAppBar(
                title = { Text(stringResource(if (state.isNew) R.string.edit_new_title else R.string.edit_title)) },
                subtitle = state.form.name.takeIf { !state.isNew && it.isNotBlank() }?.let { { Text(it) } },
                navigationIcon = {
                    IconButton(onClick = requestClose) {
                        Icon(FiendIcons.Close, contentDescription = stringResource(R.string.edit_close))
                    }
                },
                actions = {
                    Button(
                        onClick = onSave,
                        enabled = !state.saving && state.loaded,
                        modifier = Modifier.padding(end = 12.dp),
                    ) { Text(stringResource(R.string.edit_save)) }
                },
                scrollBehavior = scrollBehavior,
            )
        },
    ) { padding ->
        if (!state.loaded) return@Scaffold
        val f = state.form
        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(padding)
                .imePadding()
                .padding(horizontal = 16.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Column(Modifier.widthIn(max = 560.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                GroupHeader(stringResource(R.string.edit_group_general))
                FormField(
                    value = f.name,
                    onValueChange = { v -> onChange { it.copy(name = v) } },
                    label = stringResource(R.string.edit_name),
                    icon = FiendIcons.Flag,
                    error = state.errors[Field.Name],
                    capitalization = KeyboardCapitalization.Words,
                )
                GroupHeader(stringResource(R.string.edit_group_server))
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
                    value = f.secret,
                    onValueChange = { v -> onChange { it.copy(secret = v.trim().take(64)) } },
                    label = stringResource(R.string.edit_secret),
                    icon = FiendIcons.Key,
                    error = state.errors[Field.Secret],
                    supporting = "${stringResource(R.string.edit_secret_hint)} · ${f.secret.length}/64",
                    keyboardType = KeyboardType.Password,
                    visualTransformation = if (secretVisible) VisualTransformation.None else PasswordVisualTransformation(),
                    trailing = {
                        IconButton(onClick = { secretVisible = !secretVisible }) {
                            Icon(
                                if (secretVisible) FiendIcons.VisibilityOff else FiendIcons.Visibility,
                                contentDescription = stringResource(if (secretVisible) R.string.detail_hide_secret else R.string.detail_show_secret),
                            )
                        }
                    },
                )
                GroupHeader(stringResource(R.string.edit_group_fingerprint))
                TemplatePicker(state, onChange)

                AdvancedHeader(open = advancedOpen, onToggle = { advancedOpen = !advancedOpen })
                AnimatedVisibility(visible = advancedOpen) {
                    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            NumberField(f.handshake, { v -> onChange { it.copy(handshake = v) } }, R.string.edit_handshake, state.errors[Field.Handshake], Modifier.weight(1f))
                            NumberField(f.connections, { v -> onChange { it.copy(connections = v) } }, R.string.edit_connections, state.errors[Field.Connections], Modifier.weight(1f))
                        }
                        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            NumberField(f.udpIdle, { v -> onChange { it.copy(udpIdle = v) } }, R.string.edit_udp_idle, state.errors[Field.UdpIdle], Modifier.weight(1f))
                            NumberField(f.udpKeepalive, { v -> onChange { it.copy(udpKeepalive = v) } }, R.string.edit_udp_keepalive, state.errors[Field.UdpKeepalive], Modifier.weight(1f))
                        }
                    }
                }
                Spacer(Modifier.height(32.dp))
            }
        }
    }

    if (confirmDiscard) {
        AlertDialog(
            onDismissRequest = { confirmDiscard = false },
            title = { Text(stringResource(R.string.edit_discard_title)) },
            text = { Text(stringResource(R.string.edit_discard_body)) },
            confirmButton = {
                TextButton(onClick = {
                    confirmDiscard = false
                    onClose()
                }) { Text(stringResource(R.string.edit_discard)) }
            },
            dismissButton = {
                TextButton(onClick = { confirmDiscard = false }) { Text(stringResource(R.string.edit_keep)) }
            },
        )
    }
}

@Composable
private fun errorText(error: FieldError?): String? = error?.let {
    stringResource(
        when (it) {
            FieldError.Required -> R.string.edit_error_required
            FieldError.Port -> R.string.edit_error_port
            FieldError.Secret -> R.string.edit_error_secret
            FieldError.Number -> R.string.edit_error_number
            FieldError.Keepalive -> R.string.edit_error_keepalive
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
    val message = errorText(error) ?: supporting
    OutlinedTextField(
        value = value,
        onValueChange = onValueChange,
        label = { Text(label) },
        leadingIcon = { Icon(icon, contentDescription = null) },
        trailingIcon = trailing,
        isError = error != null,
        supportingText = message?.let { { Text(it) } },
        singleLine = true,
        visualTransformation = visualTransformation,
        keyboardOptions = KeyboardOptions(keyboardType = keyboardType, capitalization = capitalization),
        shape = MaterialTheme.shapes.large,
        modifier = Modifier.fillMaxWidth(),
    )
}

@Composable
private fun NumberField(value: String, onValueChange: (String) -> Unit, label: Int, error: FieldError?, modifier: Modifier) {
    OutlinedTextField(
        value = value,
        onValueChange = { onValueChange(it.filter(Char::isDigit).take(7)) },
        label = { Text(stringResource(label)) },
        isError = error != null,
        supportingText = errorText(error)?.let { { Text(it) } },
        singleLine = true,
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
        shape = MaterialTheme.shapes.large,
        modifier = modifier,
    )
}

@Composable
private fun TemplatePicker(state: ProfileEditUiState, onChange: ((ProfileForm) -> ProfileForm) -> Unit) {
    var expanded by rememberSaveable { mutableStateOf(false) }
    val selected = state.templates.firstOrNull { it.id == state.form.templateId }
    ExposedDropdownMenuBox(expanded = expanded, onExpandedChange = { expanded = it }) {
        OutlinedTextField(
            value = selected?.let { "${it.name} · ${it.platform}" }.orEmpty(),
            onValueChange = {},
            readOnly = true,
            label = { Text(stringResource(R.string.edit_template)) },
            leadingIcon = { Icon(FiendIcons.Fingerprint, contentDescription = null) },
            trailingIcon = { Icon(FiendIcons.ExpandMore, contentDescription = null, modifier = Modifier.rotate(if (expanded) 180f else 0f)) },
            supportingText = { Text(stringResource(R.string.edit_template_hint)) },
            singleLine = true,
            shape = MaterialTheme.shapes.large,
            modifier = Modifier
                .fillMaxWidth()
                .menuAnchor(ExposedDropdownMenuAnchorType.PrimaryNotEditable),
        )
        ExposedDropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
            state.templates.forEach { template ->
                DropdownMenuItem(
                    text = { Text("${template.name} · ${template.platform}") },
                    leadingIcon = if (template.id == state.form.templateId) {
                        { Icon(FiendIcons.Check, contentDescription = null) }
                    } else {
                        null
                    },
                    onClick = {
                        expanded = false
                        onChange { it.copy(templateId = template.id) }
                    },
                )
            }
        }
    }
}

@Composable
private fun AdvancedHeader(open: Boolean, onToggle: () -> Unit) {
    val rotation by animateFloatAsState(if (open) 180f else 0f, MaterialTheme.motionScheme.defaultSpatialSpec(), label = "chevron")
    SegmentedListItem(
        onClick = onToggle,
        shapes = ListItemDefaults.segmentedShapes(0, 1),
        colors = FiendListDefaults.colors(),
        leadingContent = { Icon(FiendIcons.Tune, contentDescription = null) },
        trailingContent = { Icon(FiendIcons.ExpandMore, contentDescription = null, modifier = Modifier.rotate(rotation)) },
        modifier = Modifier.padding(top = 12.dp),
    ) {
        Text(stringResource(R.string.edit_group_advanced))
    }
}
