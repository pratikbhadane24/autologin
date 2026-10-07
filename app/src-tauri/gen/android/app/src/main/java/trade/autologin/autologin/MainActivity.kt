package trade.autologin.autologin

import android.content.Intent
import android.os.Bundle
import android.view.View
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    keepClearOfSystemBars(findViewById(android.R.id.content))
    takeRunRequest(intent)
  }

  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    takeRunRequest(intent)
  }

  // Opened by the daily-login alarm or its notification: Rust picks the
  // request up (app/phone_schedule.rs). The extra is removed so a rotation
  // or relaunch doesn't start a second run.
  private fun takeRunRequest(intent: Intent?) {
    if (intent?.getBooleanExtra(LoginAlarms.EXTRA_RUN, false) != true) return
    intent.removeExtra(LoginAlarms.EXTRA_RUN)
    LoginAlarms.requestRun(this)
  }

  // The WebView reports no safe-area insets to CSS, so pad the page natively
  // to keep it out from under the status bar, navigation bar and keyboard.
  private fun keepClearOfSystemBars(content: View) {
    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val bars = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or
          WindowInsetsCompat.Type.displayCutout() or
          WindowInsetsCompat.Type.ime()
      )
      view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
      WindowInsetsCompat.CONSUMED
    }
  }
}
