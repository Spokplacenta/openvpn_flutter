#include "openvpn_flutter_plugin.h"

#include <flutter/method_channel.h>
#include <flutter/event_channel.h>
#include <flutter/plugin_registrar_windows.h>
#include <flutter/standard_method_codec.h>
#include <windows.h>

#include <memory>
#include <sstream>
#include <map>
#include <string>
#include <optional>
#include <cstring>

// VpnState structure from Rust
struct VpnState {
    const char* stage;
    const char* connected_on;
    uint64_t byte_in;
    uint64_t byte_out;
    uint64_t packets_in;
    uint64_t packets_out;
};

// FFI Rust function declarations
extern "C" {
    int openvpn_initialize(const char* binary_path);
    int openvpn_connect(const char* config, const char* username, const char* password);
    int openvpn_disconnect();
    char* openvpn_get_stage();
    VpnState* openvpn_get_status();
    void openvpn_free_string(char* ptr);
    void openvpn_free_state(VpnState* ptr);
}

namespace flutter {

// static
void OpenvpnFlutterPlugin::RegisterWithRegistrar(
    PluginRegistrarWindows *registrar) {
  auto plugin = std::make_unique<OpenvpnFlutterPlugin>();
  
  plugin->method_channel_ =
      std::make_unique<MethodChannel<EncodableValue>>(
          registrar->messenger(), "id.laskarmedia.openvpn_flutter/vpncontrol",
          &StandardMethodCodec::GetInstance());

  plugin->method_channel_->SetMethodCallHandler(
      [plugin_pointer = plugin.get()](const auto &call, auto result) {
        plugin_pointer->HandleMethodCall(call, std::move(result));
      });

  plugin->SetupEventChannel(registrar);
  
  registrar->AddPlugin(std::move(plugin));
}

OpenvpnFlutterPlugin::OpenvpnFlutterPlugin() : last_emitted_stage_("disconnected") {}

OpenvpnFlutterPlugin::~OpenvpnFlutterPlugin() {}

void OpenvpnFlutterPlugin::SetupEventChannel(
    PluginRegistrarWindows *registrar) {
  event_channel_ =
      std::make_unique<EventChannel<EncodableValue>>(
          registrar->messenger(), "id.laskarmedia.openvpn_flutter/vpnstage",
          &StandardMethodCodec::GetInstance());

  auto event_handler = std::make_unique<
      StreamHandler<EncodableValue>>(
      [this](const EncodableValue* arguments,
             std::unique_ptr<EventSink<EncodableValue>>&& events)
          -> std::unique_ptr<StreamHandlerError<EncodableValue>> {
        this->event_sink_ = std::move(events);
        return nullptr;
      },
      [this](const EncodableValue* arguments)
          -> std::unique_ptr<StreamHandlerError<EncodableValue>> {
        this->event_sink_.reset();
        return nullptr;
      });

  event_channel_->SetStreamHandler(std::move(event_handler));
}

void OpenvpnFlutterPlugin::HandleMethodCall(
    const MethodCall<EncodableValue> &method_call,
    std::unique_ptr<MethodResult<EncodableValue>> result) {
  
  const std::string& method = method_call.method_name();

  if (method == "initialize") {
    const auto* args = std::get_if<EncodableMap>(method_call.arguments());
    if (!args) {
      result->Error("InvalidArguments", "Expected map");
      return;
    }

    // For Windows, we need the path to openvpn.exe
    // We can get it from WindowsBinaryManager on the Dart side
    // Note: iOS parameters (groupIdentifier, etc.) are not required for Windows
    std::string binary_path;
    auto it = args->find(EncodableValue("binaryPath"));
    if (it != args->end()) {
      const auto* path_value = std::get_if<std::string>(&it->second);
      if (path_value) {
        binary_path = *path_value;
      }
    }

    if (binary_path.empty()) {
      result->Error("InvalidArguments", "binaryPath is required for Windows");
      return;
    }

    int ret = openvpn_initialize(binary_path.c_str());
    if (ret == 0) {
      result->Success(EncodableValue("disconnected"));
    } else {
      std::string error_msg = "Failed to initialize OpenVPN (error code: " + std::to_string(ret) + ")";
      result->Error("InitializationFailed", error_msg);
    }
  } else if (method == "connect") {
    const auto* args = std::get_if<EncodableMap>(method_call.arguments());
    if (!args) {
      result->Error("InvalidArguments", "Expected map");
      return;
    }

    std::string config;
    std::string username;
    std::string password;

    auto config_it = args->find(EncodableValue("config"));
    if (config_it != args->end()) {
      const auto* config_value = std::get_if<std::string>(&config_it->second);
      if (config_value) {
        config = *config_value;
      }
    }

    auto username_it = args->find(EncodableValue("username"));
    if (username_it != args->end()) {
      const auto* username_value = std::get_if<std::string>(&username_it->second);
      if (username_value) {
        username = *username_value;
      }
    }

    auto password_it = args->find(EncodableValue("password"));
    if (password_it != args->end()) {
      const auto* password_value = std::get_if<std::string>(&password_it->second);
      if (password_value) {
        password = *password_value;
      }
    }

    if (config.empty()) {
      result->Error("InvalidArguments", "config is required");
      return;
    }

    int ret = openvpn_connect(
        config.c_str(),
        username.empty() ? nullptr : username.c_str(),
        password.empty() ? nullptr : password.c_str()
    );

    if (ret == 0) {
      result->Success(nullptr);
      // Check and emit current stage
      EmitCurrentStage();
    } else {
      std::string error_msg = "Failed to connect (error code: " + std::to_string(ret) + ")";
      result->Error("ConnectionFailed", error_msg);
    }
  } else if (method == "disconnect") {
    int ret = openvpn_disconnect();
    if (ret == 0) {
      result->Success(nullptr);
      // Emit a disconnection event
      if (event_sink_) {
        event_sink_->Success(EncodableValue("disconnected"));
      }
    } else {
      std::string error_msg = "Failed to disconnect (error code: " + std::to_string(ret) + ")";
      result->Error("DisconnectFailed", error_msg);
    }
  } else if (method == "stage") {
    char* stage = openvpn_get_stage();
    if (stage) {
      std::string stage_str(stage);
      openvpn_free_string(stage);
      result->Success(EncodableValue(stage_str));
      // Emit stage if different from last emitted
      if (stage_str != last_emitted_stage_) {
        last_emitted_stage_ = stage_str;
        if (event_sink_) {
          event_sink_->Success(EncodableValue(last_emitted_stage_));
        }
      }
    } else {
      result->Success(EncodableValue("disconnected"));
    }
  } else if (method == "status") {
    // Get status from Rust
    VpnState* state = openvpn_get_status();
    if (state) {
      // Build JSON
      std::ostringstream json;
      json << "{";
      
      // connected_on
      if (state->connected_on && strlen(state->connected_on) > 0) {
        json << "\"connected_on\":\"" << state->connected_on << "\",";
      } else {
        json << "\"connected_on\":null,";
      }
      
      // Statistics
      json << "\"byte_in\":\"" << state->byte_in << "\",";
      json << "\"byte_out\":\"" << state->byte_out << "\",";
      json << "\"packets_in\":\"" << state->packets_in << "\",";
      json << "\"packets_out\":\"" << state->packets_out << "\"";
      
      json << "}";
      
      std::string status_json = json.str();
      
      // Free the structure
      openvpn_free_state(state);
      
      result->Success(EncodableValue(status_json));
    } else {
      // On error, return empty status
      std::string status_json = "{\"connected_on\":null,\"byte_in\":\"0\",\"byte_out\":\"0\",\"packets_in\":\"0\",\"packets_out\":\"0\"}";
      result->Success(EncodableValue(status_json));
    }
  } else {
    result->NotImplemented();
  }
}

void OpenvpnFlutterPlugin::EmitCurrentStage() {
  if (!event_sink_) {
    return;
  }
  
  char* stage = openvpn_get_stage();
  if (stage) {
    std::string stage_str(stage);
    openvpn_free_string(stage);
    
    // Emit only if different from last emitted
    if (stage_str != last_emitted_stage_) {
      last_emitted_stage_ = stage_str;
      event_sink_->Success(EncodableValue(stage_str));
    }
  }
}

}  // namespace flutter

void OpenvpnFlutterPluginRegisterWithRegistrar(
    FlutterDesktopPluginRegistrarRef registrar) {
  flutter::OpenvpnFlutterPlugin::RegisterWithRegistrar(
      flutter::PluginRegistrarManager::GetInstance()
          ->GetRegistrar<flutter::PluginRegistrarWindows>(registrar));
}

