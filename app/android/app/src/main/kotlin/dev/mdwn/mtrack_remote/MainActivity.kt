// Copyright (C) 2026 Michael Wilson <mike@mdwn.dev>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, version 3.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License along with
// this program. If not, see <https://www.gnu.org/licenses/>.
//
package dev.mdwn.mtrack_remote

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.Bundle
import android.util.Log
import io.flutter.embedding.android.FlutterActivity

/**
 * The app's one activity. Beside hosting Flutter it keeps the process's
 * sockets on Wi-Fi while the phone has a Wi-Fi network.
 *
 * A player's own network has no internet. Android then makes mobile data the
 * default network, and a socket the app opens to the player leaves by mobile
 * data and reaches nothing. The app talks to nothing but the player (and to
 * itself on 127.0.0.1, which a binding does not affect), so the whole process
 * is bound to Wi-Fi whenever there is one, whether or not it has internet,
 * and unbound when it goes.
 */
class MainActivity : FlutterActivity() {
    @Volatile private var connectivity: ConnectivityManager? = null
    @Volatile private var bound: Network? = null

    private val wifi = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            bind(network)
        }

        override fun onLost(network: Network) {
            if (network == bound) {
                bind(null)
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val manager = getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
            ?: return
        // A request of its own, not the default one: the default asks for
        // internet, and would never name the player's network.
        val request = NetworkRequest.Builder()
            .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
            .removeCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .build()
        // Set first: the callback runs on another thread and may fire at once.
        connectivity = manager
        try {
            manager.registerNetworkCallback(request, wifi)
        } catch (e: RuntimeException) {
            // Without the callback the app is where it was before: Wi-Fi
            // works where it is the default network, Bluetooth elsewhere.
            connectivity = null
            Log.w(TAG, "Cannot watch for Wi-Fi", e)
        }
    }

    override fun onDestroy() {
        connectivity?.let {
            try {
                it.unregisterNetworkCallback(wifi)
            } catch (e: RuntimeException) {
                Log.w(TAG, "Cannot stop watching for Wi-Fi", e)
            }
            if (bound != null) {
                it.bindProcessToNetwork(null)
                bound = null
            }
        }
        connectivity = null
        super.onDestroy()
    }

    private fun bind(network: Network?) {
        val manager = connectivity ?: return
        if (manager.bindProcessToNetwork(network)) {
            bound = network
            Log.i(TAG, if (network == null) "Sockets follow the system" else "Sockets bound to Wi-Fi")
        } else {
            // Seen with a VPN up that apps may not bypass. The sockets then
            // go where the system sends them, as before.
            Log.w(TAG, "Cannot bind sockets to Wi-Fi; they follow the system")
        }
    }

    private companion object {
        const val TAG = "mtrack"
    }
}
