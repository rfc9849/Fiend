package dev.fiend.feature.profiles.list

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.foundation.text.input.clearText
import androidx.compose.foundation.text.input.rememberTextFieldState
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AppBarWithSearch
import androidx.compose.material3.ExpandedFullScreenSearchBar
import androidx.compose.material3.FloatingActionButtonMenu
import androidx.compose.material3.FloatingActionButtonMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SearchBarDefaults
import androidx.compose.material3.SearchBarState
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.ToggleFloatingActionButton
import androidx.compose.material3.ToggleFloatingActionButtonDefaults.animateIcon
import androidx.compose.material3.rememberSearchBarState
import androidx.compose.material3.toShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import android.content.Context
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.ui.platform.LocalContext
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.fiend.core.designsystem.component.GroupHeader
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.core.model.Profile
import dev.fiend.feature.profiles.R
import dev.fiend.feature.profiles.component.ProfileListItem
import kotlinx.coroutines.launch

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
    val textFieldState = rememberTextFieldState()
    val searchBarState = rememberSearchBarState()
    val scope = rememberCoroutineScope()
    var fabExpanded by rememberSaveable { mutableStateOf(false) }
    var pasteOpen by rememberSaveable { mutableStateOf(false) }
    var scanOpen by rememberSaveable { mutableStateOf(false) }
    val resources = LocalResources.current
    val context = LocalContext.current
    val openFile = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri != null) scope.launch {
            runCatching { withContext(Dispatchers.IO) { readShareFile(context, uri) } }
                .onSuccess(onImport)
                .onFailure { snackbar.showSnackbar(resources.getString(R.string.profiles_import_failed)) }
        }
    }

    LaunchedEffect(textFieldState) {
        snapshotFlow { textFieldState.text.toString() }.collect(onQueryChange)
    }
    BackHandler(fabExpanded) { fabExpanded = false }

    val inputField = @Composable {
        SearchInput(textFieldState, searchBarState)
    }

    Box(Modifier.fillMaxSize()) {
        Scaffold(
            topBar = {
                AppBarWithSearch(
                    state = searchBarState,
                    inputField = inputField,
                )
                ExpandedFullScreenSearchBar(state = searchBarState, inputField = inputField) {
                    SearchResults(
                        query = state.query,
                        results = state.results,
                        connectedId = state.connectedId,
                        onOpen = { id ->
                            scope.launch { searchBarState.animateToCollapsed() }
                            onOpenProfile(id)
                        },
                    )
                }
            },
            snackbarHost = { SnackbarHost(snackbar, Modifier.padding(bottom = 80.dp)) },
        ) { padding ->
            if (state.loaded && state.active == null && state.saved.isEmpty()) {
                EmptyState(Modifier.padding(padding))
            } else {
                ProfilesList(state, padding, onOpenProfile)
            }
        }
        AnimatedVisibility(visible = fabExpanded, enter = fadeIn(), exit = fadeOut()) {
            Box(
                Modifier
                    .fillMaxSize()
                    .background(MaterialTheme.colorScheme.scrim.copy(alpha = 0.32f))
                    .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) {
                        fabExpanded = false
                    },
            )
        }
        FloatingActionButtonMenu(
            expanded = fabExpanded,
            modifier = Modifier
                .align(Alignment.BottomEnd)
                .windowInsetsPadding(WindowInsets.safeDrawing.only(WindowInsetsSides.End + WindowInsetsSides.Bottom))
                .padding(end = 16.dp, bottom = 16.dp),
            button = {
                ToggleFloatingActionButton(
                    checked = fabExpanded,
                    onCheckedChange = { fabExpanded = it },
                    modifier = Modifier.semantics {
                        contentDescription = resources.getString(R.string.profiles_add)
                        stateDescription = if (fabExpanded) "expanded" else "collapsed"
                    },
                ) {
                    val icon = if (checkedProgress > 0.5f) FiendIcons.Close else FiendIcons.Add
                    Icon(icon, contentDescription = null, modifier = Modifier.animateIcon({ checkedProgress }))
                }
            },
        ) {
            AddAction.entries.forEach { action ->
                FloatingActionButtonMenuItem(
                    onClick = {
                        fabExpanded = false
                        when (action) {
                            AddAction.Scan -> scanOpen = true
                            AddAction.Paste -> pasteOpen = true
                            AddAction.File -> openFile.launch(arrayOf("*/*"))
                            AddAction.Manual -> onAddManually()
                        }
                    },
                    icon = { Icon(action.icon(), contentDescription = null) },
                    text = { Text(stringResource(action.label)) },
                )
            }
        }
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
private fun SearchInput(textFieldState: TextFieldState, searchBarState: SearchBarState) {
    SearchBarDefaults.InputField(
        textFieldState = textFieldState,
        searchBarState = searchBarState,
        onSearch = {},
        placeholder = { Text(stringResource(R.string.profiles_search)) },
        leadingIcon = { Icon(FiendIcons.Search, contentDescription = null) },
        trailingIcon = if (textFieldState.text.isNotEmpty()) {
            {
                IconButton(onClick = { textFieldState.clearText() }) {
                    Icon(FiendIcons.Close, contentDescription = stringResource(R.string.profiles_clear_search))
                }
            }
        } else {
            null
        },
    )
}

@Composable
private fun ProfilesList(state: ProfilesUiState, padding: PaddingValues, onOpen: (String) -> Unit) {
    LazyColumn(
        contentPadding = PaddingValues(
            start = 16.dp,
            end = 16.dp,
            top = padding.calculateTopPadding(),
            bottom = padding.calculateBottomPadding() + 96.dp,
        ),
        modifier = Modifier.fillMaxSize(),
    ) {
        state.active?.let { active ->
            item(key = "h-active") { GroupHeader(stringResource(R.string.profiles_section_active)) }
            item(key = active.id) {
                ProfileListItem(
                    profile = active,
                    shapes = ListItemDefaults.segmentedShapes(0, 1),
                    connected = state.connectedId == active.id,
                    highlighted = true,
                    onClick = { onOpen(active.id) },
                    modifier = Modifier.animateItem(),
                )
            }
        }
        if (state.saved.isNotEmpty()) {
            item(key = "h-saved") { GroupHeader(stringResource(R.string.profiles_section_saved)) }
            itemsIndexed(state.saved, key = { _, it -> it.id }) { index, profile ->
                ProfileListItem(
                    profile = profile,
                    shapes = ListItemDefaults.segmentedShapes(index, state.saved.size),
                    connected = state.connectedId == profile.id,
                    onClick = { onOpen(profile.id) },
                    modifier = Modifier
                        .animateItem()
                        .padding(bottom = ListItemDefaults.SegmentedGap),
                )
            }
        }
    }
}

@Composable
private fun SearchResults(query: String, results: List<Profile>, connectedId: String?, onOpen: (String) -> Unit) {
    if (results.isEmpty()) {
        Text(
            text = stringResource(R.string.profiles_no_results, query),
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(24.dp),
        )
        return
    }
    LazyColumn(
        contentPadding = PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap),
    ) {
        itemsIndexed(results, key = { _, it -> it.id }) { index, profile ->
            ProfileListItem(
                profile = profile,
                shapes = ListItemDefaults.segmentedShapes(index, results.size),
                connected = connectedId == profile.id,
                onClick = { onOpen(profile.id) },
            )
        }
    }
}

@Composable
private fun EmptyState(modifier: Modifier = Modifier) {
    Column(
        modifier = modifier
            .fillMaxSize()
            .padding(32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Box(
            Modifier
                .size(120.dp)
                .clip(MaterialShapes.Cookie9Sided.toShape())
                .background(MaterialTheme.colorScheme.secondaryContainer),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                FiendIcons.Dns,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSecondaryContainer,
                modifier = Modifier.size(48.dp),
            )
        }
        Spacer(Modifier.height(24.dp))
        Text(stringResource(R.string.profiles_empty_title), style = MaterialTheme.typography.headlineSmallEmphasized)
        Spacer(Modifier.height(8.dp))
        Text(
            stringResource(R.string.profiles_empty_body),
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
    }
}

@Composable
private fun PasteLinkDialog(onDismiss: () -> Unit, onImport: (String) -> Unit) {
    var link by rememberSaveable { mutableStateOf("") }
    val clipboard = LocalClipboard.current
    val scope = rememberCoroutineScope()
    AlertDialog(
        onDismissRequest = onDismiss,
        icon = { Icon(FiendIcons.AddLink, contentDescription = null) },
        title = { Text(stringResource(R.string.profiles_paste_title)) },
        text = {
            Column {
                Text(stringResource(R.string.profiles_paste_body))
                Spacer(Modifier.height(16.dp))
                OutlinedTextField(
                    value = link,
                    onValueChange = { link = it },
                    label = { Text(stringResource(R.string.profiles_paste_label)) },
                    placeholder = { Text("fiend://…") },
                    singleLine = true,
                    trailingIcon = {
                        IconButton(onClick = {
                            scope.launch {
                                clipboard.getClipEntry()?.clipData?.takeIf { it.itemCount > 0 }
                                    ?.getItemAt(0)?.text?.let { link = it.toString() }
                            }
                        }) {
                            Icon(FiendIcons.ContentPaste, contentDescription = stringResource(R.string.profiles_paste_from_clipboard))
                        }
                    },
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        },
        confirmButton = {
            TextButton(onClick = { onImport(link) }, enabled = link.isNotBlank()) {
                Text(stringResource(R.string.profiles_import))
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.profiles_cancel)) }
        },
    )
}
