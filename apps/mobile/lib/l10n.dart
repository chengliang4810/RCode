import 'package:flutter/widgets.dart';

extension RCodeLocalization on BuildContext {
  bool get isChinese => Localizations.localeOf(this).languageCode == 'zh';
  String tr(String chinese, String english) => isChinese ? chinese : english;
}
