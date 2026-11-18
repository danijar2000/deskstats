package app.deskstats

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.util.Log
import android.widget.Toast

/**
 * Stores the server's port and the token sent over adb, then opens the panel. Only adb's shell
 * may start it (the manifest requires the DUMP permission), so other apps cannot redirect the panel.
 *
 *   am start -n app.deskstats/.ConfigActivity --ei port 8787 --es token <token>
 */
class ConfigActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val prefs = getSharedPreferences(Settings.PREFS, Context.MODE_PRIVATE)
        val problem = store(prefs)
        if (problem != null) {
            // Said aloud and logged: a setting dropped in silence leaves the owner guessing.
            Log.w("deskstats", "ConfigActivity: $problem")
            Toast.makeText(this, "DeskStats: $problem", Toast.LENGTH_LONG).show()
        }
        startActivity(Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        finish()
    }

    /** Stores what the intent brought; returns what is wrong with it, or null. */
    private fun store(prefs: android.content.SharedPreferences): String? {
        // Before Android 4.1 any app could get DUMP, so there a token, once stored, stays.
        if (Build.VERSION.SDK_INT < 16 && !prefs.getString(Settings.TOKEN, null).isNullOrEmpty()) {
            return "токен уже сохранён; чтобы связать заново, очистите данные приложения"
        }
        // Without a port the stored one stays.
        val port = intent.getIntExtra("port", prefs.getInt(Settings.USB_PORT, Settings.DEFAULT_PORT))
        val token = intent.getStringExtra("token")?.trim()
        if (port !in 1..65535) return "порт $port не подходит"
        if (token != null && (token.length !in 32..128 || !token.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' })) {
            return "токен не подходит: нужно от 32 до 128 шестнадцатеричных знаков"
        }
        val edit = prefs.edit().putInt(Settings.USB_PORT, port)
        if (token != null) edit.putString(Settings.TOKEN, token)
        // The panel reads the settings as soon as it comes up, hence commit().
        edit.commit()
        return null
    }
}

object Settings {
    const val PREFS = "deskstats"
    const val USB_PORT = "usb_port"
    const val TOKEN = "token"
    const val WIFI_HOST = "wifi_host"
    const val WIFI_PORT = "wifi_port"
    const val DEFAULT_PORT = 8787

    /** An IPv4 address of a home or office network; the Mac is never looked for elsewhere. */
    fun isPrivateAddress(host: String): Boolean {
        val parts = host.split('.').map { it.toIntOrNull() ?: return false }
        if (parts.size != 4 || parts.any { it !in 0..255 }) return false
        return parts[0] == 10 ||
            (parts[0] == 192 && parts[1] == 168) ||
            (parts[0] == 172 && parts[1] in 16..31)
    }
}

/** Opens the panel after the phone restarts. */
class BootReceiver : android.content.BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED) return
        context.startActivity(Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }
}
