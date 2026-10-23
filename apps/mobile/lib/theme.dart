import 'package:flutter/cupertino.dart' show CupertinoPageTransitionsBuilder;
import 'package:flutter/material.dart';

@immutable
class RCodeColors extends ThemeExtension<RCodeColors> {
  const RCodeColors({
    required this.background,
    required this.elevated,
    required this.text,
    required this.secondary,
    required this.tertiary,
    required this.hairline,
    required this.accent,
    required this.danger,
    required this.success,
    required this.warning,
    required this.userBubble,
    required this.codeBackground,
    required this.chip,
  });

  final Color background,
      elevated,
      text,
      secondary,
      tertiary,
      hairline,
      accent,
      danger,
      success,
      warning,
      userBubble,
      codeBackground,
      chip;

  static const light = RCodeColors(
    background: Color(0xfff3f3f5),
    elevated: Color(0xffffffff),
    text: Color(0xff27272c),
    secondary: Color(0xff62626a),
    tertiary: Color(0xff97979f),
    hairline: Color(0xffe2e2e6),
    accent: Color(0xff5b43e8),
    danger: Color(0xffdc2626),
    success: Color(0xff15803d),
    warning: Color(0xffa16207),
    userBubble: Color(0xffffffff),
    codeBackground: Color(0xfffafafb),
    chip: Color(0xffe7e7eb),
  );
  static const dark = RCodeColors(
    background: Color(0xff060606),
    elevated: Color(0xff111113),
    text: Color(0xffe8e8ea),
    secondary: Color(0xffa9a9ae),
    tertiary: Color(0xff6b6b72),
    hairline: Color(0xff1e1e22),
    accent: Color(0xff8b7cf6),
    danger: Color(0xfff87171),
    success: Color(0xff34d399),
    warning: Color(0xfffacc15),
    userBubble: Color(0xff19191c),
    codeBackground: Color(0xff0b0b0d),
    chip: Color(0xff1c1c20),
  );

  @override
  RCodeColors copyWith() => this;

  @override
  RCodeColors lerp(covariant RCodeColors? other, double t) {
    if (other == null) return this;
    Color mix(Color a, Color b) => Color.lerp(a, b, t)!;
    return RCodeColors(
      background: mix(background, other.background),
      elevated: mix(elevated, other.elevated),
      text: mix(text, other.text),
      secondary: mix(secondary, other.secondary),
      tertiary: mix(tertiary, other.tertiary),
      hairline: mix(hairline, other.hairline),
      accent: mix(accent, other.accent),
      danger: mix(danger, other.danger),
      success: mix(success, other.success),
      warning: mix(warning, other.warning),
      userBubble: mix(userBubble, other.userBubble),
      codeBackground: mix(codeBackground, other.codeBackground),
      chip: mix(chip, other.chip),
    );
  }
}

extension RCodeThemeContext on BuildContext {
  RCodeColors get colors => Theme.of(this).extension<RCodeColors>()!;
}

const rcodeFontFallbacks = [
  'PingFang SC',
  'Noto Sans CJK SC',
  'Microsoft YaHei',
];

final _lightTheme = _buildTheme(Brightness.light);
final _darkTheme = _buildTheme(Brightness.dark);

ThemeData rcodeTheme(Brightness brightness) =>
    brightness == Brightness.dark ? _darkTheme : _lightTheme;

ThemeData _buildTheme(Brightness brightness) {
  final c = brightness == Brightness.dark
      ? RCodeColors.dark
      : RCodeColors.light;
  final scheme = ColorScheme.fromSeed(
    seedColor: c.accent,
    brightness: brightness,
    surface: c.background,
    onSurface: c.text,
    error: c.danger,
  );
  return ThemeData(
    useMaterial3: true,
    brightness: brightness,
    colorScheme: scheme,
    fontFamily: 'Geist',
    fontFamilyFallback: rcodeFontFallbacks,
    scaffoldBackgroundColor: c.background,
    extensions: [c],
    splashFactory: NoSplash.splashFactory,
    appBarTheme: AppBarTheme(
      backgroundColor: c.background,
      foregroundColor: c.text,
      surfaceTintColor: Colors.transparent,
      elevation: 0,
      centerTitle: true,
      titleTextStyle: TextStyle(
        fontFamily: 'Geist',
        fontFamilyFallback: rcodeFontFallbacks,
        fontSize: 17,
        fontWeight: FontWeight.w600,
        color: c.text,
      ),
    ),
    dividerTheme: DividerThemeData(color: c.hairline, thickness: 0.5, space: 1),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: c.elevated,
      contentPadding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
      border: OutlineInputBorder(
        borderRadius: BorderRadius.circular(16),
        borderSide: BorderSide.none,
      ),
      enabledBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(16),
        borderSide: BorderSide.none,
      ),
      focusedBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(16),
        borderSide: BorderSide(color: c.accent),
      ),
    ),
    textButtonTheme: TextButtonThemeData(
      style: TextButton.styleFrom(foregroundColor: c.accent),
    ),
    filledButtonTheme: FilledButtonThemeData(
      style: FilledButton.styleFrom(
        backgroundColor: c.accent,
        foregroundColor: Colors.white,
        padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 14),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(18)),
      ),
    ),
    bottomSheetTheme: BottomSheetThemeData(
      backgroundColor: c.background,
      surfaceTintColor: Colors.transparent,
      showDragHandle: true,
      shape: const RoundedRectangleBorder(
        borderRadius: BorderRadius.vertical(top: Radius.circular(28)),
      ),
    ),
    dialogTheme: DialogThemeData(
      backgroundColor: c.elevated,
      surfaceTintColor: Colors.transparent,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(24)),
    ),
    listTileTheme: ListTileThemeData(
      contentPadding: const EdgeInsets.symmetric(horizontal: 20),
      iconColor: c.secondary,
      textColor: c.text,
    ),
    navigationBarTheme: NavigationBarThemeData(
      backgroundColor: c.background,
      surfaceTintColor: Colors.transparent,
      indicatorColor: Colors.transparent,
      height: 66,
      labelTextStyle: WidgetStateProperty.resolveWith(
        (states) => TextStyle(
          fontSize: 11,
          color: states.contains(WidgetState.selected) ? c.text : c.secondary,
        ),
      ),
    ),
    pageTransitionsTheme: const PageTransitionsTheme(
      builders: {
        TargetPlatform.iOS: CupertinoPageTransitionsBuilder(),
        TargetPlatform.android: CupertinoPageTransitionsBuilder(),
      },
    ),
  );
}
