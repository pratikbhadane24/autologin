package trade.autologin.autologin

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import androidx.core.app.ActivityCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class ScheduleArgs {
  var times: List<Long> = emptyList()
  var automatic: Boolean = false
}

@InvokeArg
class SettingArgs {
  lateinit var which: String
}

/** Rust's side of the phone schedule (app/phone_schedule.rs). */
@TauriPlugin
class LoginSchedulePlugin(private val activity: Activity) : Plugin(activity) {
  private var asked = false

  @Command
  fun setSchedule(invoke: Invoke) {
    val args = invoke.parseArgs(ScheduleArgs::class.java)
    LoginAlarms.save(activity, args.times, args.automatic)
    invoke.resolve()
  }

  @Command
  fun takePendingRun(invoke: Invoke) {
    invoke.resolve(JSObject().put("value", LoginAlarms.takePendingRun(activity)))
  }

  /** What the automatic mode still needs, for the setup checklist. */
  @Command
  fun status(invoke: Invoke) {
    val power = activity.getSystemService(PowerManager::class.java)
    invoke.resolve(
      JSObject()
        .put("notifications", LoginAlarms.canNotify(activity))
        .put("exactAlarms", LoginAlarms.canScheduleExact(activity))
        .put("overlay", Settings.canDrawOverlays(activity))
        .put("batteryUnrestricted", power.isIgnoringBatteryOptimizations(activity.packageName))
    )
  }

  /** Opens the system screen where the user grants one of the above. */
  @Command
  fun openSetting(invoke: Invoke) {
    val args = invoke.parseArgs(SettingArgs::class.java)
    val app = Uri.parse("package:${activity.packageName}")
    val intent = when (args.which) {
      "notifications" -> {
        val permission = Manifest.permission.POST_NOTIFICATIONS
        // Android stops showing the prompt after it's been declined twice;
        // then only the app's notification settings can turn it on.
        val canPrompt = Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
          (!asked || ActivityCompat.shouldShowRequestPermissionRationale(activity, permission))
        if (canPrompt) {
          asked = true
          ActivityCompat.requestPermissions(activity, arrayOf(permission), 0)
          return invoke.resolve()
        }
        Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, activity.packageName)
      }
      "exactAlarms" -> Intent(Settings.ACTION_REQUEST_SCHEDULE_EXACT_ALARM, app)
      "overlay" -> Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, app)
      "battery" -> Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)
      else -> return invoke.reject("unknown setting: ${args.which}")
    }
    try {
      activity.startActivity(intent)
      invoke.resolve()
    } catch (e: Exception) {
      invoke.reject("couldn't open that setting: ${e.message}")
    }
  }
}
