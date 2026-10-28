package dev.fiend.feature.home

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Text
import androidx.compose.material3.SheetValue
import androidx.compose.material3.rememberBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import dev.fiend.core.designsystem.component.FiendListDefaults
import dev.fiend.core.designsystem.component.ShapeAvatar
import dev.fiend.core.designsystem.component.avatarLabel
import dev.fiend.core.designsystem.component.latencyColor
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.core.model.Profile

@Composable
internal fun ProfilePickerSheet(
    profiles: List<Profile>,
    selectedId: String?,
    onSelect: (String) -> Unit,
    onManage: () -> Unit,
    onDismiss: () -> Unit,
) {
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberBottomSheetState(initialValue = SheetValue.Hidden, enabledValues = setOf(SheetValue.Hidden, SheetValue.Expanded)),
        containerColor = MaterialTheme.colorScheme.surfaceContainerLow,
    ) {
        Column(Modifier.navigationBarsPadding()) {
            Text(
                text = stringResource(R.string.home_picker_title),
                style = MaterialTheme.typography.titleLarge,
                modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp),
            )
            Spacer(Modifier.height(8.dp))
            LazyColumn(
                verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap),
                modifier = Modifier.padding(horizontal = 16.dp),
            ) {
                itemsIndexed(profiles, key = { _, it -> it.id }) { index, profile ->
                    val selected = profile.id == selectedId
                    SegmentedListItem(
                        selected = selected,
                        onClick = { onSelect(profile.id) },
                        shapes = ListItemDefaults.segmentedShapes(index = index, count = profiles.size),
                        colors = FiendListDefaults.colors(containerColor = MaterialTheme.colorScheme.surfaceContainerHigh),
                        leadingContent = {
                            ShapeAvatar(label = avatarLabel(profile.name, profile.countryCode), seed = profile.id)
                        },
                        supportingContent = { Text(profile.authority, maxLines = 1) },
                        trailingContent = {
                            if (selected) {
                                Icon(FiendIcons.CheckCircleFilled, contentDescription = null, tint = MaterialTheme.colorScheme.primary)
                            } else {
                                profile.latencyMs?.let {
                                    Text(
                                        stringResource(R.string.home_latency, it),
                                        style = MaterialTheme.typography.labelLarge,
                                        color = latencyColor(it),
                                    )
                                }
                            }
                        },
                    ) {
                        Text(profile.name)
                    }
                }
            }
            FilledTonalButton(
                onClick = onManage,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 16.dp)
                    .height(56.dp),
            ) {
                Icon(FiendIcons.Tune, contentDescription = null)
                Spacer(Modifier.padding(4.dp))
                Text(stringResource(R.string.home_picker_manage), style = MaterialTheme.typography.titleSmall)
            }
        }
    }
}
