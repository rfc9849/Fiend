package dev.fiend.feature.settings

import android.text.format.DateFormat
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LargeFlexibleTopAppBar
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import dev.fiend.core.designsystem.component.ActionItem
import dev.fiend.core.designsystem.component.SegmentedGroup
import dev.fiend.core.designsystem.component.SwitchItem
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.core.model.AppSettings
import dev.fiend.core.model.RuleKind
import java.util.Date

private enum class RoutingDialog { Resolver, Mtu, Subscription }

@Composable
internal fun RoutingScreen(
    state: RoutingUiState,
    onBack: () -> Unit,
    onOpenDirect: () -> Unit,
    onOpenBlock: () -> Unit,
    onOpenProxy: () -> Unit,
    onDirectMode: (Boolean) -> Unit,
    onResolver: (String) -> RoutingInputError?,
    onMtu: (String) -> RoutingInputError?,
    onAddSubscription: (String, RuleKind) -> RoutingInputError?,
    onRefreshSubscription: (String) -> Unit,
    onRemoveSubscription: (String) -> Unit,
    onRefreshAll: () -> Unit,
    onMessageShown: () -> Unit,
) {
    val scrollBehavior = TopAppBarDefaults.exitUntilCollapsedScrollBehavior()
    val snackbar = remember { SnackbarHostState() }
    var dialog by rememberSaveable { mutableStateOf<RoutingDialog?>(null) }
    val enabled = state.loaded && !state.busy

    LaunchedEffect(state.message) {
        state.message?.let {
            snackbar.showSnackbar(it)
            onMessageShown()
        }
    }

    Scaffold(
        modifier = Modifier.nestedScroll(scrollBehavior.nestedScrollConnection),
        snackbarHost = { SnackbarHost(snackbar) },
        topBar = {
            LargeFlexibleTopAppBar(
                title = { Text(stringResource(R.string.routing_title)) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(FiendIcons.ArrowBack, contentDescription = stringResource(R.string.split_back))
                    }
                },
                scrollBehavior = scrollBehavior,
            )
        },
    ) { padding ->
        LazyColumn(
            contentPadding = PaddingValues(
                start = 16.dp,
                end = 16.dp,
                top = padding.calculateTopPadding(),
                bottom = padding.calculateBottomPadding() + 24.dp,
            ),
            modifier = Modifier.fillMaxSize(),
        ) {
            item {
                SegmentedGroup(title = stringResource(R.string.routing_group_rules)) {
                    item {
                        ActionItem(
                            shapes = it,
                            headline = stringResource(R.string.routing_direct_rules),
                            supporting = ruleCount(state.directRules),
                            icon = FiendIcons.AltRoute,
                            trailing = { Icon(FiendIcons.ChevronRight, contentDescription = null) },
                            enabled = state.loaded,
                            onClick = onOpenDirect,
                        )
                    }
                    item {
                        ActionItem(
                            shapes = it,
                            headline = stringResource(R.string.routing_block_rules),
                            supporting = ruleCount(state.blockRules),
                            icon = FiendIcons.Block,
                            trailing = { Icon(FiendIcons.ChevronRight, contentDescription = null) },
                            enabled = state.loaded,
                            onClick = onOpenBlock,
                        )
                    }
                    item {
                        ActionItem(
                            shapes = it,
                            headline = stringResource(R.string.routing_proxy_rules),
                            supporting = ruleCount(state.proxyRules),
                            icon = FiendIcons.NorthEast,
                            trailing = { Icon(FiendIcons.ChevronRight, contentDescription = null) },
                            enabled = state.loaded,
                            onClick = onOpenProxy,
                        )
                    }
                }
            }
            item {
                Text(
                    text = stringResource(R.string.routing_privacy),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 8.dp, vertical = 16.dp),
                )
            }
            item {
                SegmentedGroup(title = stringResource(R.string.routing_group_subscriptions)) {
                    state.subscriptions.forEach { subscription ->
                        item {
                            ActionItem(
                                shapes = it,
                                headline = subscription.url,
                                supporting = subscriptionSummary(subscription),
                                icon = FiendIcons.Link,
                                trailing = {
                                    Row {
                                        IconButton(
                                            onClick = { onRefreshSubscription(subscription.id) },
                                            enabled = enabled,
                                        ) {
                                            Icon(
                                                FiendIcons.RestartAlt,
                                                contentDescription = stringResource(R.string.routing_subscription_refresh),
                                            )
                                        }
                                        IconButton(
                                            onClick = { onRemoveSubscription(subscription.id) },
                                            enabled = enabled,
                                        ) {
                                            Icon(
                                                FiendIcons.Delete,
                                                contentDescription = stringResource(R.string.routing_subscription_remove),
                                            )
                                        }
                                    }
                                },
                                enabled = enabled,
                                onClick = { onRefreshSubscription(subscription.id) },
                            )
                        }
                    }
                    item {
                        ActionItem(
                            shapes = it,
                            headline = stringResource(R.string.routing_subscription_add),
                            supporting = stringResource(R.string.routing_subscription_add_body),
                            icon = FiendIcons.Add,
                            enabled = enabled,
                            onClick = { dialog = RoutingDialog.Subscription },
                        )
                    }
                    if (state.subscriptions.isNotEmpty()) {
                        item {
                            ActionItem(
                                shapes = it,
                                headline = stringResource(R.string.routing_subscription_refresh_all),
                                icon = FiendIcons.RestartAlt,
                                enabled = enabled,
                                onClick = onRefreshAll,
                            )
                        }
                    }
                }
            }
            if (state.busy) {
                item {
                    LinearProgressIndicator(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 8.dp, vertical = 16.dp),
                    )
                }
            }
            item {
                SegmentedGroup(title = stringResource(R.string.routing_group_network)) {
                    item {
                        ActionItem(
                            shapes = it,
                            headline = stringResource(R.string.routing_resolver),
                            supporting = state.directResolver.ifBlank { stringResource(R.string.routing_resolver_default) },
                            icon = FiendIcons.Dns,
                            enabled = state.loaded,
                            onClick = { dialog = RoutingDialog.Resolver },
                        )
                    }
                    item {
                        ActionItem(
                            shapes = it,
                            headline = stringResource(R.string.routing_mtu),
                            supporting = state.mtu.toString(),
                            icon = FiendIcons.Speed,
                            enabled = state.loaded,
                            onClick = { dialog = RoutingDialog.Mtu },
                        )
                    }
                    item {
                        SwitchItem(
                            shapes = it,
                            headline = stringResource(R.string.routing_direct_mode),
                            supporting = stringResource(R.string.routing_direct_mode_body),
                            icon = FiendIcons.Public,
                            checked = state.directMode,
                            enabled = state.loaded,
                            onCheckedChange = onDirectMode,
                        )
                    }
                }
            }
            item {
                Text(
                    text = stringResource(R.string.routing_restart),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 8.dp, vertical = 16.dp),
                )
            }
        }
    }

    when (dialog) {
        RoutingDialog.Resolver -> ValueDialog(
            title = stringResource(R.string.routing_resolver),
            initial = state.directResolver,
            label = stringResource(R.string.routing_resolver),
            hint = stringResource(R.string.routing_resolver_body),
            errorText = stringResource(R.string.routing_error_resolver),
            keyboardType = KeyboardType.Uri,
            onConfirm = onResolver,
            onDismiss = { dialog = null },
        )
        RoutingDialog.Mtu -> ValueDialog(
            title = stringResource(R.string.routing_mtu),
            initial = state.mtu.toString(),
            label = stringResource(R.string.routing_mtu),
            hint = stringResource(R.string.routing_mtu_body, AppSettings.MIN_MTU, AppSettings.MAX_MTU),
            errorText = stringResource(R.string.routing_error_mtu, AppSettings.MIN_MTU, AppSettings.MAX_MTU),
            keyboardType = KeyboardType.Number,
            onConfirm = onMtu,
            onDismiss = { dialog = null },
        )
        RoutingDialog.Subscription -> SubscriptionDialog(
            onConfirm = onAddSubscription,
            onDismiss = { dialog = null },
        )
        null -> Unit
    }
}

@Composable
private fun ruleCount(count: Int): String =
    if (count == 0) stringResource(R.string.routing_rules_none) else pluralStringResource(R.plurals.routing_rules, count, count)

@Composable
private fun kindLabel(kind: RuleKind): String = stringResource(
    when (kind) {
        RuleKind.Direct -> R.string.routing_kind_direct
        RuleKind.Block -> R.string.routing_kind_block
        RuleKind.Proxy -> R.string.routing_kind_proxy
    },
)

@Composable
private fun subscriptionSummary(subscription: SubscriptionUi): String {
    if (subscription.error != null) {
        return stringResource(R.string.routing_subscription_failed, subscription.error)
    }
    val kind = kindLabel(subscription.kind)
    val entries = pluralStringResource(R.plurals.routing_rules, subscription.entries, subscription.entries)
    val updated = if (subscription.updatedAtMs == 0L) {
        stringResource(R.string.routing_subscription_never)
    } else {
        val context = LocalContext.current
        val formatter = remember(context) { DateFormat.getMediumDateFormat(context) }
        formatter.format(Date(subscription.updatedAtMs))
    }
    return stringResource(R.string.routing_subscription_summary, kind, entries, updated)
}

@Composable
private fun SubscriptionDialog(
    onConfirm: (String, RuleKind) -> RoutingInputError?,
    onDismiss: () -> Unit,
) {
    var url by rememberSaveable { mutableStateOf("") }
    var kind by rememberSaveable { mutableStateOf(RuleKind.Direct) }
    var failed by remember { mutableStateOf(false) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.routing_subscription_add)) },
        text = {
            Column {
                OutlinedTextField(
                    value = url,
                    onValueChange = { url = it; failed = false },
                    label = { Text(stringResource(R.string.routing_subscription_url)) },
                    isError = failed,
                    supportingText = {
                        Text(if (failed) stringResource(R.string.routing_error_link) else stringResource(R.string.routing_subscription_url_body))
                    },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                    shape = MaterialTheme.shapes.large,
                    modifier = Modifier.fillMaxWidth(),
                )
                Text(
                    text = stringResource(R.string.routing_subscription_kind),
                    style = MaterialTheme.typography.labelLarge,
                    modifier = Modifier.padding(top = 16.dp, bottom = 8.dp),
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    RuleKind.entries.forEach { option ->
                        FilterChip(
                            selected = kind == option,
                            onClick = { kind = option },
                            label = { Text(kindLabel(option)) },
                        )
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = {
                if (onConfirm(url, kind) == null) onDismiss() else failed = true
            }) { Text(stringResource(R.string.routing_subscription_add_action)) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.settings_cancel)) }
        },
    )
}

@Composable
private fun ValueDialog(
    title: String,
    initial: String,
    label: String,
    hint: String,
    errorText: String,
    keyboardType: KeyboardType,
    onConfirm: (String) -> RoutingInputError?,
    onDismiss: () -> Unit,
) {
    var value by rememberSaveable { mutableStateOf(initial) }
    var failed by remember { mutableStateOf(false) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            OutlinedTextField(
                value = value,
                onValueChange = { value = it; failed = false },
                label = { Text(label) },
                isError = failed,
                supportingText = { Text(if (failed) errorText else hint) },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = keyboardType),
                shape = MaterialTheme.shapes.large,
                modifier = Modifier.fillMaxWidth(),
            )
        },
        confirmButton = {
            TextButton(onClick = {
                if (onConfirm(value) == null) onDismiss() else failed = true
            }) { Text(stringResource(R.string.routing_save)) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.settings_cancel)) }
        },
    )
}
