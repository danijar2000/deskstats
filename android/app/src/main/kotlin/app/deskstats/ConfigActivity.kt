package app.deskstats

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.util.Log
import android.widget.Toast

class ConfigActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val prefs = getSharedPreferences(Settings.PREFS, Context.MODE_PRIVATE)
        val problem = store(prefs)
        if (problem != null) {
            val ui = Ui.localize(this, Options.load(prefs).language)
            val message = ui.getString(problem.first, problem.second)
            Log.w("deskstats", "ConfigActivity: " + resources.getResourceEntryName(problem.first))
            Toast.makeText(this, ui.getString(R.string.config_problem, message), Toast.LENGTH_LONG).show()
        }
        startActivity(Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        finish()
    }

    private fun store(prefs: android.content.SharedPreferences): Pair<Int, Int>? {
        val port = intent.getIntExtra("port", prefs.getInt(Settings.USB_PORT, Settings.DEFAULT_PORT))
        val token = intent.getStringExtra("token")?.trim()
        if (port !in 1..65535) return R.string.config_bad_port to port
        if (token != null && !Servers.validToken(token)) return R.string.config_bad_token to 0
        // Before Android 4.1 any app may start this activity, so once a Mac is paired only that Mac is listened to.
        val known = Servers.load(prefs).list
        if (Build.VERSION.SDK_INT < 16 && known.isNotEmpty() && (token == null || known.none { it.id == Server.idOf(token) })) {
            return R.string.config_token_kept to 0
        }
        if (token != null) Servers.update(prefs) { it.withToken(token) }
        prefs.edit().putInt(Settings.USB_PORT, port).commit()
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

    fun isPrivateAddress(host: String): Boolean {
        val parts = host.split('.').map { it.toIntOrNull() ?: return false }
        if (parts.size != 4 || parts.any { it !in 0..255 }) return false
        return parts[0] == 10 ||
            (parts[0] == 192 && parts[1] == 168) ||
            (parts[0] == 172 && parts[1] in 16..31)
    }
}

class BootReceiver : android.content.BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED) return
        context.startActivity(Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }
}
