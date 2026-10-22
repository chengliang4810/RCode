import 'visual_backend.dart';

Future<VisualBackend> openVisual(Map<String, dynamic> config) => Future.error(
  UnsupportedError('Visuals require the mobile native renderer'),
);

Future<Map<String, dynamic>?> renderNativeMath(
  Map<String, dynamic> config,
) async => null;
