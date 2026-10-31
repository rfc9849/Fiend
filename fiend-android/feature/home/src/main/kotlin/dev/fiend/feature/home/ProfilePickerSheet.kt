package dev.fiend.feature.home

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.SheetValue
import androidx.compose.material3.Text
import androidx.compose.material3.rememberBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.fiend.core.designsystem.component.FiendButton
import dev.fiend.core.designsystem.component.FiendButtonKind
import dev.fiend.core.designsystem.component.ShapeAvatar
import dev.fiend.core.designsystem.component.avatarLabel
import dev.fiend.core.designsystem.component.latencyColor
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.core.designsystem.theme.FiendColors
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
        containerColor = FiendColors.panel,
        contentColor = FiendColors.text,
        shape = RoundedCornerShape(topStart = 9.dp, topEnd = 9.dp),
        dragHandle = {
            Spacer(
                Modifier
                    .padding(top = 10.dp, bottom = 6.dp)
                    .size(width = 36.dp, height = 4.dp)
                    .background(FiendColors.line, RoundedCornerShape(2.dp)),
            )
        },
    ) {
        Column(Modifier.navigationBarsPadding()) {
            Text(
                text = stringResource(R.string.home_picker_title),
                style = MaterialTheme.typography.titleLarge,
                color = FiendColors.text,
                modifier = Modifier.padding(horizontal = 20.dp, vertical = 8.dp),
            )
            Spacer(Modifier.height(8.dp))
            LazyColumn(modifier = Modifier.fillMaxWidth()) {
                itemsIndexed(profiles, key = { _, it -> it.id }) { index, profile ->
                    if (index > 0) HorizontalDivider(color = FiendColors.line, thickness = 1.dp)
                    PickerRow(
                        profile = profile,
                        selected = profile.id == selectedId,
                        onClick = { onSelect(profile.id) },
                    )
                }
            }
            FiendButton(
                label = stringResource(R.string.home_picker_manage),
                onClick = onManage,
                kind = FiendButtonKind.Primary,
                icon = FiendIcons.Tune,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 16.dp),
            )
        }
    }
}

@Composable
private fun PickerRow(profile: Profile, selected: Boolean, onClick: () -> Unit) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 72.dp)
            .clickable(onClick = onClick)
            .padding(horizontal = 20.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        ShapeAvatar(label = avatarLabel(profile.name, profile.countryCode), seed = profile.id)
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.Center) {
            Text(
                text = profile.name,
                style = MaterialTheme.typography.titleMedium,
                color = FiendColors.text,
                maxLines = 1,
            )
            Text(
                text = profile.authority,
                style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace, fontSize = 12.sp),
                color = FiendColors.muted,
                maxLines = 1,
            )
        }
        if (selected) {
            Icon(FiendIcons.Check, contentDescription = null, tint = FiendColors.accent, modifier = Modifier.size(20.dp))
        } else {
            profile.latencyMs?.let {
                Text(
                    text = stringResource(R.string.home_latency, it),
                    style = MaterialTheme.typography.labelMedium,
                    color = latencyColor(it),
                )
            }
        }
    }
}
