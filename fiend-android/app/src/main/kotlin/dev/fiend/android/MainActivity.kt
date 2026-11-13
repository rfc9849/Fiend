package dev.fiend.android

import android.os.Bundle
import android.os.Build
import android.app.Activity
import android.Manifest
import android.content.pm.PackageManager
import android.net.VpnService
import android.content.Intent
import android.widget.Toast
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.receiveAsFlow
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.ContextCompat
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.core.splashscreen.SplashScreen.Companion.installSplashScreen
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dagger.hilt.android.AndroidEntryPoint
import dagger.hilt.android.lifecycle.HiltViewModel
import dev.fiend.android.ui.FiendApp
import dev.fiend.core.data.SettingsRepository
import dev.fiend.core.data.ProfileRepository
import dev.fiend.core.data.SubscriptionRepository
import dev.fiend.core.data.TunnelController
import dev.fiend.core.model.TunnelState
import dev.fiend.core.designsystem.theme.FiendTheme
import dev.fiend.core.model.AppSettings
import dev.fiend.core.model.ThemeMode
import javax.inject.Inject
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.launch
import androidx.lifecycle.viewModelScope

@HiltViewModel
class MainViewModel @Inject constructor(
    settings: SettingsRepository,
    private val profiles: ProfileRepository,
    tunnel: TunnelController,
    private val subscriptions: SubscriptionRepository,
) : ViewModel() {
    val settings: StateFlow<AppSettings> = settings.settings
    private val messages = Channel<String>(Channel.BUFFERED)
    val importMessages = messages.receiveAsFlow()
    fun importLink(link: String) {
        viewModelScope.launch {
            profiles.import(link).fold(
                onSuccess = { messages.send("Profile imported: ${it.name}") },
                onFailure = { messages.send(it.message ?: "Cannot import the profile") },
            )
        }
    }
    init {
        viewModelScope.launch { runCatching { subscriptions.refreshStale() } }
        viewModelScope.launch {
            settings.settings.filter { it.connectOnStart }.first()
            val list = profiles.profiles.first { it.isNotEmpty() }
            val id = profiles.activeProfileId.value ?: list.first().id
            if (tunnel.state.value == TunnelState.Disconnected) tunnel.connect(id)
        }
    }
}

@AndroidEntryPoint
class MainActivity : ComponentActivity() {
    private val viewModel: MainViewModel by viewModels()
    @Inject lateinit var tunnel: TunnelController
    private val consent = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) {
        tunnel.permissionResult(it.resultCode == Activity.RESULT_OK)
    }
    private val notifications = registerForActivityResult(ActivityResultContracts.RequestPermission()) { }
    private val localNetwork = registerForActivityResult(ActivityResultContracts.RequestPermission()) {
        if (it) requestVpnConsent() else tunnel.permissionResult(false)
    }

    private fun requestVpnConsent() {
        val request = if (viewModel.settings.value.rootMode) null else VpnService.prepare(this)
        if (request == null) tunnel.permissionResult(true) else consent.launch(request)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        intent.data?.takeIf { it.scheme == "fiend" }?.let { viewModel.importLink(it.toString()) }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        installSplashScreen()
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        if (savedInstanceState == null) intent?.data?.takeIf { it.scheme == "fiend" }?.let { viewModel.importLink(it.toString()) }
        setContent {
            LaunchedEffect(viewModel) { viewModel.importMessages.collect { Toast.makeText(this@MainActivity, it, Toast.LENGTH_LONG).show() } }
            val settings by viewModel.settings.collectAsStateWithLifecycle()
            val permissionRequired by tunnel.permissionRequired.collectAsStateWithLifecycle()
            val tunnelState by tunnel.state.collectAsStateWithLifecycle()
            var consentOpen by rememberSaveable { mutableStateOf(false) }
            var notificationsAsked by rememberSaveable { mutableStateOf(false) }
            LaunchedEffect(permissionRequired) {
                if (!permissionRequired) consentOpen = false
                else if (!consentOpen) {
                    consentOpen = true
                    if (Build.VERSION.SDK_INT >= 37 && tunnel.localNetworkPermissionNeeded()) localNetwork.launch(Manifest.permission.ACCESS_LOCAL_NETWORK)
                    else requestVpnConsent()
                }
            }
            LaunchedEffect(tunnelState) {
                if (tunnelState is TunnelState.Connected && Build.VERSION.SDK_INT >= 33 && !notificationsAsked &&
                    ContextCompat.checkSelfPermission(this@MainActivity, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
                    notificationsAsked = true
                    notifications.launch(Manifest.permission.POST_NOTIFICATIONS)
                }
            }
            val dark = when (settings.themeMode) {
                ThemeMode.System -> isSystemInDarkTheme()
                ThemeMode.Light -> false
                ThemeMode.Dark -> true
            }
            FiendTheme(darkTheme = dark) {
                FiendApp(versionName = "${BuildConfig.VERSION_NAME} · core 0.1.8")
            }
        }
    }
}
