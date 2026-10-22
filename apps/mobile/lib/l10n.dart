import 'package:flutter/widgets.dart';
import 'package:flutter/services.dart' show PlatformException;

import 'data/rpc_client.dart' show RpcException;

extension RCodeLocalization on BuildContext {
  bool get isChinese => Localizations.localeOf(this).languageCode == 'zh';
  String tr(String chinese, String english) => isChinese ? chinese : english;

  String effortLabel(String level) => switch (level) {
    'none' => tr('关闭', 'None'),
    'minimal' => tr('最低', 'Minimal'),
    'low' => tr('低', 'Low'),
    'medium' => tr('中', 'Medium'),
    'high' => tr('高', 'High'),
    'xhigh' => tr('超高', 'Extra high'),
    'max' => tr('最高', 'Maximum'),
    'auto' => tr('自动', 'Auto'),
    _ => level,
  };

  String statusLabel(String status) => switch (status) {
    'running' || 'working' => tr('正在运行', 'Running'),
    'done' || 'complete' || 'completed' => tr('已完成', 'Completed'),
    'failed' || 'errored' => tr('失败', 'Failed'),
    'cancelled' || 'canceled' => tr('已取消', 'Cancelled'),
    'awaitingInput' => tr('等待回答', 'Waiting for input'),
    'idle' => tr('空闲', 'Idle'),
    _ => status,
  };

  String platformLabel(String platform) => switch (platform.toLowerCase()) {
    'windows' => 'Windows',
    'macos' || 'darwin' => 'macOS',
    'linux' => 'Linux',
    'ios' => 'iOS',
    'android' => 'Android',
    '' || 'unknown' => tr('未知', 'Unknown'),
    _ => platform,
  };

  /// Translate application error codes, retaining server/user-authored details.
  String errorText(Object? error) {
    if (error == null) return '';
    final raw = switch (error) {
      RpcException e => e.message,
      PlatformException e => e.message ?? e.code,
      FormatException e => e.message,
      StateError e => e.message,
      _ => error.toString(),
    };
    final code = switch (error) {
      RpcException e => e.code,
      PlatformException e => e.code,
      _ => raw,
    };
    final messages = <String, (String, String)>{
      'invalid_address': ('请输入有效的服务器地址', 'Enter a valid server address'),
      'invalid_key': (
        'API 密钥至少 8 位，且同时包含字母和数字',
        'The API key needs at least 8 letters and digits',
      ),
      'unauthorized': ('API 密钥无效', 'The API key is invalid'),
      'forbidden': ('访问被拒绝，请检查 API 密钥', 'Access denied. Check the API key'),
      'connection_failed': ('无法连接服务器', 'Could not connect to the server'),
      'server_disconnected': ('服务器连接已断开', 'Disconnected from the server'),
      'disconnected': ('服务器连接已断开', 'Disconnected from the server'),
      'operation_unconfirmed': (
        '操作结果尚未确认，请重新连接后检查',
        'The result is unconfirmed. Reconnect and check',
      ),
      'unconfirmed': (
        '操作结果尚未确认，请重新连接后检查',
        'The result is unconfirmed. Reconnect and check',
      ),
      'operation_rejected': ('操作已被服务器拒绝', 'The server rejected this operation'),
      'operation_failed': ('操作失败，请重试', 'The operation failed. Try again'),
      'secure_storage_unavailable': (
        '无法访问系统安全存储',
        'System secure storage is unavailable',
      ),
      'session_unavailable': ('会话暂不可用', 'The conversation is unavailable'),
      'session_closed': ('会话已关闭', 'The conversation is closed'),
      'empty_message': ('请输入消息或添加图片', 'Enter a message or add an image'),
      'send_pending': ('正在发送，请稍候', 'A message is being sent'),
      'attachment_too_large': (
        '单张图片不能超过 24 MiB',
        'Each image must be smaller than 24 MiB',
      ),
      'image_too_large': (
        '单张图片不能超过 24 MiB',
        'Each image must be smaller than 24 MiB',
      ),
      'attachment_upload_failed': (
        '图片上传失败，请重试',
        'Image upload failed. Try again',
      ),
      'attachment_read_failed': (
        '图片读取失败，请重试',
        'Could not read the image. Try again',
      ),
      'Unsupported recovered attachment': (
        '恢复的附件不是图片',
        'The recovered attachment is not an image',
      ),
      'Unsupported image format': ('不支持此图片格式', 'Unsupported image format'),
      'Image conversion failed': ('图片转换失败', 'Image conversion failed'),
      'Invalid clipboard image': (
        '无法读取剪贴板中的图片',
        'Could not read the clipboard image',
      ),
      'clipboard_read_failed': (
        '无法读取剪贴板中的图片',
        'Could not read the clipboard image',
      ),
      'Editing lease expired': (
        '编辑权限已失效，请重新打开消息',
        'Editing expired. Reopen the message',
      ),
    };
    final translated = messages[raw] ?? messages[code];
    return translated == null ? raw : tr(translated.$1, translated.$2);
  }
}
