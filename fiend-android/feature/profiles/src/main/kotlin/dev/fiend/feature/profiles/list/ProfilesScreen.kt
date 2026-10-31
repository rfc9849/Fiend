package dev.fiend.feature.profiles.list

import android.content.Context
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.fiend.core.designsystem.component.FiendAlertDialog
import dev.fiend.core.designsystem.component.FiendButton
import dev.fiend.core.designsystem.component.FiendButtonKind
import dev.fiend.core.designsystem.component.FiendGroup
import dev.fiend.core.designsystem.component.FiendIconButton
import dev.fiend.core.designsystem.component.FiendTextField
import dev.fiend.core.designsystem.component.FiendTopBar
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.core.designsystem.theme.FiendColors
import dev.fiend.core.model.Profile
import dev.fiend.feature.profiles.R
import dev.fiend.feature.profiles.component.ProfileListItem
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@Composable
internal fun ProfilesRoute(
    onOpenProfile: (String) -> Unit,
    onAddManually: () -> Unit,
    viewModel: ProfilesViewModel = androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel(),
) {
    val state by viewModel.uiState.collectAsStateWithLifecycle()
    val snackbar = remember { SnackbarHostState() }
    val resources = LocalResources.current
    LaunchedEffect(viewModel) {
        viewModel.events.collect { event ->
            val message = when (event) {
                is ProfilesEvent.Imported -> resources.getString(R.string.profiles_imported, event.name)
                ProfilesEvent.ImportFailed -> resources.getString(R.string.profiles_import_failed)
            }
            snackbar.showSnackbar(message)
        }
    }
    ProfilesScreen(
        state = state,
        snackbar = snackbar,
        onQueryChange = viewModel::onQueryChange,
        onOpenProfile = onOpenProfile,
        onAddManually = onAddManually,
        onImport = viewModel::import,
    )
}

private enum class AddAction(val label: Int) {
    Scan(R.string.profiles_add_scan),
    Paste(R.string.profiles_add_paste),
    File(R.string.profiles_add_file),
    Manual(R.string.profiles_add_manual),
}

@Composable
private fun AddAction.icon(): ImageVector = when (this) {
    AddAction.Scan -> FiendIcons.QrCodeScanner
    AddAction.Paste -> FiendIcons.ContentPaste
    AddAction.File -> FiendIcons.UploadFile
    AddAction.Manual -> FiendIcons.Edit
}

@Composable
internal fun ProfilesScreen(
    state: ProfilesUiState,
    snackbar: SnackbarHostState,
    onQueryChange: (String) -> Unit,
    onOpenProfile: (String) -> Unit,
    onAddManually: () -> Unit,
    onImport: (String) -> Unit,
) {
    var addOpen by rememberSaveable { mutableStateOf(false) }
    var pasteOpen by rememberSaveable { mutableStateOf(false) }
    var scanOpen by rememberSaveable { mutableStateOf(false) }
    val resources = LocalResources.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val openFile = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri != null) scope.launch {
            runCatching { withContext(Dispatchers.IO) { readShareFile(context, uri) } }
                .onSuccess(onImport)
                .onFailure { snackbar.showSnackbar(resources.getString(R.string.profiles_import_failed)) }
        }
    }

    Box(Modifier.fillMaxSize().background(FiendColors.surface)) {
        Column(Modifier.fillMaxSize()) {
            FiendTopBar(
                title = stringResource(R.string.profiles_title),
                actions = {
                    Box {
                        FiendButton(
                            label = stringResource(R.string.profiles_add_short),
                            onClick = { addOpen = true },
                            icon = FiendIcons.Add,
                            kind = FiendButtonKind.Primary,
                            compact = true,
                        )
                        DropdownMenu(
                            expanded = addOpen,
                            onDismissRequest = { addOpen = false },
                            containerColor = FiendColors.panel,
                            shape = RoundedCornerShape(7.dp),
                        ) {
                            AddAction.entries.forEach { action ->
                                DropdownMenuItem(
                                    text = {
                                        Text(
                                            stringResource(action.label),
                                            style = MaterialTheme.typography.bodyMedium,
                                            color = FiendColors.text,
                                        )
                                    },
                                    leadingIcon = {
                                        Icon(action.icon(), contentDescription = null, tint = FiendColors.muted, modifier = Modifier.size(20.dp))
                                    },
                                    onClick = {
                                        addOpen = false
                                        when (action) {
                                            AddAction.Scan -> scanOpen = true
                                            AddAction.Paste -> pasteOpen = true
                                            AddAction.File -> openFile.launch(arrayOf("*/*"))
                                            AddAction.Manual -> onAddManually()
                                        }
                                    },
                                    modifier = Modifier.heightIn(min = 44.dp),
                                )
                            }
                        }
                    }
                },
            )
            Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.TopCenter) {
                FiendTextField(
                    value = state.query,
                    onValueChange = onQueryChange,
                    modifier = Modifier
                        .widthIn(max = 720.dp)
                        .padding(horizontal = 16.dp, vertical = 8.dp),
                    placeholder = stringResource(R.string.profiles_search),
                    leadingIcon = FiendIcons.Search,
                    trailing = if (state.query.isNotEmpty()) {
                        {
                            FiendIconButton(
                                icon = FiendIcons.Close,
                                label = stringResource(R.string.profiles_clear_search),
                                onClick = { onQueryChange("") },
                            )
                        }
                    } else {
                        null
                    },
                )
            }
            Box(Modifier.fillMaxWidth().weight(1f), contentAlignment = Alignment.TopCenter) {
                Column(Modifier.widthIn(max = 720.dp).fillMaxSize()) {
                    if (state.loaded && state.active == null && state.saved.isEmpty()) {
                        EmptyState(onAdd = { addOpen = true })
                    } else if (state.query.isNotBlank()) {
                        SearchResults(
                            query = state.query,
                            results = state.results,
                            connectedId = state.connectedId,
                            onOpen = onOpenProfile,
                        )
                    } else {
                        ProfilesList(state = state, onOpen = onOpenProfile)
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

    if (scanOpen) QrScanner(onResult = { scanOpen = false; onImport(it) }, onDismiss = { scanOpen = false })
    if (pasteOpen) {
        PasteLinkDialog(
            onDismiss = { pasteOpen = false },
            onImport = {
                pasteOpen = false
                onImport(it)
            },
        )
    }
}

private fun readShareFile(context: Context, uri: Uri): String = context.contentResolver.openInputStream(uri)?.use { input ->
    val bytes = ByteArray(65537)
    var used = 0
    while (used < bytes.size) {
        val size = input.read(bytes, used, bytes.size - used)
        if (size < 0) break
        used += size
    }
    require(used <= 65536) { "Share file is too large" }
    bytes.copyOf(used).toString(Charsets.UTF_8).trim()
} ?: error("Cannot open the share file")

@Composable
private fun ProfilesList(state: ProfilesUiState, onOpen: (String) -> Unit) {
    LazyColumn(
        contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 24.dp),
        modifier = Modifier.fillMaxSize(),
    ) {
        state.active?.let { active ->
            item(key = "active") {
                FiendGroup(title = stringResource(R.string.profiles_section_active)) {
                    item {
                        ProfileListItem(
                            profile = active,
                            connected = state.connectedId == active.id,
                            highlighted = true,
                            onClick = { onOpen(active.id) },
                        )
                    }
                }
            }
        }
        if (state.saved.isNotEmpty()) {
            item(key = "saved") {
                FiendGroup(title = stringResource(R.string.profiles_section_saved)) {
                    state.saved.forEach { profile ->
                        item {
                            ProfileListItem(
                                profile = profile,
                                connected = state.connectedId == profile.id,
                                onClick = { onOpen(profile.id) },
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun SearchResults(query: String, results: List<Profile>, connectedId: String?, onOpen: (String) -> Unit) {
    if (results.isEmpty()) {
        Text(
            text = stringResource(R.string.profiles_no_results, query),
            style = MaterialTheme.typography.bodyMedium,
            color = FiendColors.muted,
            modifier = Modifier.padding(24.dp),
        )
        return
    }
    LazyColumn(
        contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 24.dp),
        modifier = Modifier.fillMaxSize(),
    ) {
        item {
            FiendGroup(title = stringResource(R.string.profiles_search_results)) {
                results.forEach { profile ->
                    item {
                        ProfileListItem(
                            profile = profile,
                            connected = connectedId == profile.id,
                            onClick = { onOpen(profile.id) },
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun EmptyState(onAdd: () -> Unit) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Box(
            modifier = Modifier
                .size(64.dp)
                .background(FiendColors.soft, RoundedCornerShape(6.dp)),
            contentAlignment = Alignment.Center,
        ) {
            Icon(FiendIcons.Dns, contentDescription = null, tint = FiendColors.muted, modifier = Modifier.size(30.dp))
        }
        Spacer(Modifier.height(20.dp))
        Text(stringResource(R.string.profiles_empty_title), style = MaterialTheme.typography.headlineSmall, color = FiendColors.text)
        Spacer(Modifier.height(6.dp))
        Text(
            stringResource(R.string.profiles_empty_body),
            style = MaterialTheme.typography.bodyMedium,
            color = FiendColors.muted,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(20.dp))
        FiendButton(
            label = stringResource(R.string.profiles_add),
            onClick = onAdd,
            icon = FiendIcons.Add,
            kind = FiendButtonKind.Primary,
        )
    }
}

@Composable
private fun PasteLinkDialog(onDismiss: () -> Unit, onImport: (String) -> Unit) {
    var link by rememberSaveable { mutableStateOf("") }
    val clipboard = LocalClipboard.current
    val scope = rememberCoroutineScope()
    FiendAlertDialog(
        title = stringResource(R.string.profiles_paste_title),
        confirmLabel = stringResource(R.string.profiles_import),
        onConfirm = { onImport(link) },
        confirmEnabled = link.isNotBlank(),
        dismissLabel = stringResource(R.string.profiles_cancel),
        onDismiss = onDismiss,
    ) {
        Text(
            stringResource(R.string.profiles_paste_body),
            style = MaterialTheme.typography.bodyMedium,
            color = FiendColors.muted,
        )
        Spacer(Modifier.height(16.dp))
        FiendTextField(
            value = link,
            onValueChange = { link = it },
            label = stringResource(R.string.profiles_paste_label),
            placeholder = "fiend://…",
            trailing = {
                FiendIconButton(
                    icon = FiendIcons.ContentPaste,
                    label = stringResource(R.string.profiles_paste_from_clipboard),
                    onClick = {
                        scope.launch {
                            clipboard.getClipEntry()?.clipData?.takeIf { it.itemCount > 0 }
                                ?.getItemAt(0)?.text?.let { link = it.toString() }
                        }
                    },
                )
            },
        )
    }
}
