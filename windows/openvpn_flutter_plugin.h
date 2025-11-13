#ifndef OPENVPN_FLUTTER_PLUGIN_H_
#define OPENVPN_FLUTTER_PLUGIN_H_

#include <flutter/method-channel.h>
#include <flutter/event-channel.h>
#include <flutter/plugin-registrar-windows.h>

#include <memory>
#include <string>

namespace flutter {

class OpenvpnFlutterPlugin {
 public:
  static void RegisterWithRegistrar(PluginRegistrarWindows *registrar);

  OpenvpnFlutterPlugin();

  virtual ~OpenvpnFlutterPlugin();

 private:
  void HandleMethodCall(
      const MethodCall<EncodableValue> &method_call,
      std::unique_ptr<MethodResult<EncodableValue>> result);
  
  void SetupEventChannel(PluginRegistrarWindows *registrar);
  void EmitCurrentStage();  // Checks and emits current stage if different

  std::unique_ptr<MethodChannel<EncodableValue>> method_channel_;
  std::unique_ptr<EventChannel<EncodableValue>> event_channel_;
  std::unique_ptr<EventSink<EncodableValue>> event_sink_;
  std::string last_emitted_stage_;  // Last emitted stage to avoid duplicates
};

}  // namespace flutter

#endif  // OPENVPN_FLUTTER_PLUGIN_H_

