package dev.fiend.core.designsystem.component

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.core.designsystem.theme.FiendColors

@Composable
fun FiendTopBar(
    modifier: Modifier = Modifier,
    title: String? = null,
    brand: String? = null,
    onBack: (() -> Unit)? = null,
    backLabel: String = "Back",
    navigationIcon: ImageVector = FiendIcons.ArrowBack,
    actions: @Composable RowScope.() -> Unit = {},
) {
    Row(
        modifier = modifier
            .fillMaxWidth()
            .windowInsetsPadding(WindowInsets.statusBars)
            .heightIn(min = 56.dp)
            .padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (onBack != null) {
            FiendIconButton(icon = navigationIcon, label = backLabel, onClick = onBack)
            Spacer(Modifier.width(4.dp))
        }
        when {
            brand != null -> FiendBrand(brand)
            title != null -> Text(title, style = MaterialTheme.typography.titleLarge, color = FiendColors.text)
        }
        Spacer(Modifier.weight(1f))
        Row(horizontalArrangement = Arrangement.spacedBy(4.dp), verticalAlignment = Alignment.CenterVertically, content = actions)
    }
}

@Composable
fun FiendBrand(text: String, modifier: Modifier = Modifier) {
    Row(modifier = modifier, verticalAlignment = Alignment.CenterVertically) {
        Icon(
            FiendIcons.Brand,
            contentDescription = null,
            tint = FiendColors.text,
            modifier = Modifier.size(28.dp),
        )
        Spacer(Modifier.width(9.dp))
        Text(
            text = text,
            color = FiendColors.text,
            fontSize = 29.sp,
            lineHeight = 30.sp,
            fontWeight = FontWeight.SemiBold,
            letterSpacing = (-1.1).sp,
        )
    }
}

@Composable
fun FiendNavItem(
    selected: Boolean,
    icon: ImageVector,
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    count: Int? = null,
    showLabel: Boolean = true,
) {
    val shape = RoundedCornerShape(4.dp)
    Row(
        modifier = modifier
            .heightIn(min = 46.dp)
            .background(if (selected) FiendColors.panel else FiendColors.sidebar, shape)
            .then(
                if (selected) {
                    Modifier.border(1.dp, FiendColors.line, shape)
                } else {
                    Modifier
                },
            )
            .clickable(role = Role.Tab, onClick = onClick)
            .semantics { contentDescription = label }
            .padding(horizontal = if (showLabel) 14.dp else 0.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = if (showLabel) Arrangement.Start else Arrangement.Center,
    ) {
        Icon(
            icon,
            contentDescription = null,
            tint = if (selected) FiendColors.text else FiendColors.muted,
            modifier = Modifier.size(20.dp),
        )
        if (showLabel) {
            Spacer(Modifier.width(12.dp))
            Text(
                text = label,
                style = MaterialTheme.typography.titleSmall,
                color = if (selected) FiendColors.text else FiendColors.muted,
                fontWeight = if (selected) FontWeight.Medium else FontWeight.Normal,
                maxLines = 1,
            )
            if (count != null) {
                Spacer(Modifier.weight(1f))
                Text(
                    text = count.toString(),
                    style = MaterialTheme.typography.labelSmall,
                    color = FiendColors.muted,
                )
            }
        }
    }
}
