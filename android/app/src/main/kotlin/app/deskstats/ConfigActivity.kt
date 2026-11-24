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
        if (Build.VERSION.SDK_INT < 16 && !prefs.getString(Settings.TOKEN, null).isNullOrEmpty()) {
            return R.string.config_token_kept to 0
        }
        val port = intent.getIntExtra("port", prefs.getInt(Settings.USB_PORT, Settings.DEFAULT_PORT))
        val token = intent.getStringExtra("token")?.trim()
        if (port !in 1..65535) return R.string.config_bad_port to port
        if (token != null && (token.length !in 32..128 || !token.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' })) {
            return R.string.config_bad_token to 0
        }
        val edit = prefs.edit().putInt(Settings.USB_PORT, port)
        if (token != null) edit.putString(Settings.TOKEN, token)
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
