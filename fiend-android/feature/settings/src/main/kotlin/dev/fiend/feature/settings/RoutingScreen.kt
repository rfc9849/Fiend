package dev.fiend.feature.settings

import android.text.format.DateFormat
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.fiend.core.designsystem.component.ActionItem
import dev.fiend.core.designsystem.component.FiendAlertDialog
import dev.fiend.core.designsystem.component.FiendButton
import dev.fiend.core.designsystem.component.FiendButtonKind
import dev.fiend.core.designsystem.component.FiendGroup
import dev.fiend.core.designsystem.component.FiendIconButton
import dev.fiend.core.designsystem.component.FiendTextField
import dev.fiend.core.designsystem.component.FiendTopBar
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.core.designsystem.theme.FiendColors
import dev.fiend.core.model.AppSettings
import dev.fiend.core.model.RuleKind
import dev.fiend.core.model.RuleSubscription
import dev.fiend.core.model.urlTitle
import java.util.Date

private enum class RoutingDialog { Resolver, Mtu, Subscription }

@Composable
internal fun RoutingScreen(
    state: RoutingUiState,
    onBack: () -> Unit,
    onOpenDirect: () -> Unit,
    onOpenBlock: () -> Unit,
    onOpenProxy: () -> Unit,
    onOpenDpi: () -> Unit,
    onResolver: (String) -> RoutingInputError?,
    onMtu: (String) -> RoutingInputError?,
    onAddSubscription: (String, RuleKind, String) -> RoutingInputError?,
    onEditSubscription: (String, String, RuleKind) -> RoutingInputError?,
    onRefreshSubscription: (String) -> Unit,
    onRemoveSubscription: (String) -> Unit,
    onRefreshAll: () -> Unit,
    onMessageShown: () -> Unit,
) {
    val snackbar = remember { SnackbarHostState() }
    var dialog by rememberSaveable { mutableStateOf<RoutingDialog?>(null) }
    var editing by rememberSaveable { mutableStateOf<String?>(null) }
    val enabled = state.loaded && !state.busy

    LaunchedEffect(state.message) {
        state.message?.let {
            snackbar.showSnackbar(it)
            onMessageShown()
        }
    }

    Box(Modifier.fillMaxSize().background(FiendColors.surface)) {
        Column(Modifier.fillMaxSize()) {
            FiendTopBar(
                title = stringResource(R.string.routing_title),
                onBack = onBack,
                backLabel = stringResource(R.string.settings_back),
            )
            LazyColumn(
                contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 24.dp),
                modifier = Modifier.weight(1f).fillMaxWidth(),
            ) {
                item {
                    FiendGroup(title = stringResource(R.string.routing_group_rules)) {
                        item {
                            ActionItem(
                                headline = stringResource(R.string.routing_direct_rules),
                                supporting = ruleCount(state.directRules),
                                icon = FiendIcons.AltRoute,
                                trailing = { Icon(FiendIcons.ChevronRight, contentDescription = null, tint = FiendColors.muted) },
                                enabled = state.loaded,
                                onClick = onOpenDirect,
                            )
                        }
                        item {
                            ActionItem(
                                headline = stringResource(R.string.routing_block_rules),
                                supporting = ruleCount(state.blockRules),
                                icon = FiendIcons.Block,
                                trailing = { Icon(FiendIcons.ChevronRight, contentDescription = null, tint = FiendColors.muted) },
                                enabled = state.loaded,
                                onClick = onOpenBlock,
                            )
                        }
                        item {
                            ActionItem(
                                headline = stringResource(R.string.routing_proxy_rules),
                                supporting = ruleCount(state.proxyRules),
                                icon = FiendIcons.NorthEast,
                                trailing = { Icon(FiendIcons.ChevronRight, contentDescription = null, tint = FiendColors.muted) },
                                enabled = state.loaded,
                                onClick = onOpenProxy,
                            )
                        }
                        item {
                            ActionItem(
                                headline = stringResource(R.string.routing_dpi_rules),
                                supporting = ruleCount(state.dpiRules),
                                icon = FiendIcons.Bolt,
                                trailing = { Icon(FiendIcons.ChevronRight, contentDescription = null, tint = FiendColors.muted) },
                                enabled = state.loaded,
                                onClick = onOpenDpi,
                            )
                        }
                    }
                }
                item {
                    Text(
                        text = stringResource(R.string.routing_privacy),
                        style = MaterialTheme.typography.bodySmall,
                        color = FiendColors.muted,
                        modifier = Modifier.padding(horizontal = 4.dp, vertical = 16.dp),
                    )
                }
                item {
                    FiendGroup(title = stringResource(R.string.routing_group_subscriptions)) {
                        state.subscriptions.forEach { subscription ->
                            item {
                                ActionItem(
                                    headline = subscription.title,
                                    supporting = subscriptionSummary(subscription),
                                    icon = FiendIcons.Link,
                                    trailing = {
                                        Row {
                                            FiendIconButton(
                                                icon = FiendIcons.RestartAlt,
                                                label = stringResource(R.string.routing_subscription_refresh),
                                                onClick = { onRefreshSubscription(subscription.id) },
                                                enabled = enabled,
                                            )
                                            FiendIconButton(
                                                icon = FiendIcons.Delete,
                                                label = stringResource(R.string.routing_subscription_remove),
                                                onClick = { onRemoveSubscription(subscription.id) },
                                                enabled = enabled,
                                            )
                                        }
                                    },
                                    enabled = enabled,
                                    onClick = { editing = subscription.id },
                                )
                            }
                        }
                        item {
                            ActionItem(
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
                                .padding(horizontal = 4.dp, vertical = 16.dp),
                            color = FiendColors.accent,
                            trackColor = FiendColors.line,
                        )
                    }
                }
                item {
                    FiendGroup(title = stringResource(R.string.routing_group_network)) {
                        item {
                            ActionItem(
                                headline = stringResource(R.string.routing_resolver),
                                supporting = state.directResolver.ifBlank { stringResource(R.string.routing_resolver_default) },
                                icon = FiendIcons.Dns,
                                enabled = state.loaded,
                                onClick = { dialog = RoutingDialog.Resolver },
                            )
                        }
                        item {
                            ActionItem(
                                headline = stringResource(R.string.routing_mtu),
                                supporting = state.mtu.toString(),
                                icon = FiendIcons.Speed,
                                enabled = state.loaded,
                                onClick = { dialog = RoutingDialog.Mtu },
                            )
                        }
                    }
                }
                item {
                    Text(
                        text = stringResource(R.string.routing_restart),
                        style = MaterialTheme.typography.bodySmall,
                        color = FiendColors.muted,
                        modifier = Modifier.padding(horizontal = 4.dp, vertical = 16.dp),
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
            initial = null,
            onConfirm = onAddSubscription,
            onDismiss = { dialog = null },
        )
        null -> Unit
    }
    state.subscriptions.firstOrNull { it.id == editing }?.let { subscription ->
        SubscriptionDialog(
            initial = subscription,
            onConfirm = { _, kind, name -> onEditSubscription(subscription.id, name, kind) },
            onDismiss = { editing = null },
        )
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
        RuleKind.Dpi -> R.string.routing_kind_dpi
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
    initial: SubscriptionUi?,
    onConfirm: (String, RuleKind, String) -> RoutingInputError?,
    onDismiss: () -> Unit,
) {
    var url by rememberSaveable(initial?.id) { mutableStateOf(initial?.url ?: "") }
    var name by rememberSaveable(initial?.id) { mutableStateOf(initial?.name ?: "") }
    var kind by rememberSaveable(initial?.id) { mutableStateOf(initial?.kind ?: RuleKind.Direct) }
    var failed by remember { mutableStateOf<RoutingInputError?>(null) }
    FiendAlertDialog(
        title = stringResource(if (initial == null) R.string.routing_subscription_add else R.string.routing_subscription_edit),
        confirmLabel = stringResource(if (initial == null) R.string.routing_subscription_add_action else R.string.routing_save),
        onConfirm = {
            val error = onConfirm(url, kind, name)
            if (error == null) onDismiss() else failed = error
        },
        dismissLabel = stringResource(R.string.settings_cancel),
        onDismiss = onDismiss,
    ) {
        Column {
            if (initial == null) {
                FiendTextField(
                    value = url,
                    onValueChange = { url = it; failed = null },
                    label = stringResource(R.string.routing_subscription_url),
                    isError = failed == RoutingInputError.Link,
                    supporting = if (failed == RoutingInputError.Link) {
                        stringResource(R.string.routing_error_link)
                    } else {
                        stringResource(R.string.routing_subscription_url_body)
                    },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                )
                Spacer(Modifier.height(12.dp))
            }
            FiendTextField(
                value = name,
                onValueChange = { name = it; failed = null },
                label = stringResource(R.string.routing_subscription_name),
                placeholder = urlTitle(url.trim()).takeIf { url.isNotBlank() } ?: "",
                isError = failed == RoutingInputError.Name,
                supporting = if (failed == RoutingInputError.Name) {
                    stringResource(R.string.routing_error_name, RuleSubscription.MAX_NAME)
                } else {
                    stringResource(R.string.routing_subscription_name_body)
                },
                singleLine = true,
            )
            Text(
                text = stringResource(R.string.routing_subscription_kind),
                style = MaterialTheme.typography.labelMedium,
                color = FiendColors.muted,
                modifier = Modifier.padding(top = 14.dp, bottom = 8.dp),
            )
            FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                RuleKind.entries.forEach { option ->
                    FiendButton(
                        label = kindLabel(option),
                        onClick = { kind = option },
                        kind = if (kind == option) FiendButtonKind.Primary else FiendButtonKind.Default,
                        compact = true,
                    )
                }
            }
            if (initial != null) {
                Text(
                    text = initial.url,
                    style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace, fontSize = 12.sp),
                    color = FiendColors.muted,
                    modifier = Modifier.padding(top = 14.dp),
                )
            }
        }
    }
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
    FiendAlertDialog(
        title = title,
        confirmLabel = stringResource(R.string.routing_save),
        onConfirm = {
            if (onConfirm(value) == null) onDismiss() else failed = true
        },
        dismissLabel = stringResource(R.string.settings_cancel),
        onDismiss = onDismiss,
    ) {
        FiendTextField(
            value = value,
            onValueChange = { value = it; failed = false },
            label = label,
            isError = failed,
            supporting = if (failed) errorText else hint,
            singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = keyboardType),
        )
    }
}
