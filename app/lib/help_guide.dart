import 'package:flutter/material.dart';

const desktopFeatures = <(String, String)>[
  (
    "缓存范围",
    "本应用仅缓存指定 GBF CDN 域名下，符合路径和文件类型规则的静态资源，并非所有经过本应用的资源都会缓存。\n\n以下 CDN 域名的 HTTPS 443 端口流量参与处理：\n\n• prd-game-a-gbf.akamaized.net 及 a1～a5 分片。\n• prd-game-a-granbluefantasy.akamaized.net 及 a1～a5 分片。\n• prd-game-a-granbluefantasy-steam.akamaized.net 及 a1～a5 分片。\n• granbluefantasy.akamaized.net、gbf.akamaized.net。\n\n缓存范围限于 /assets/、/assets_en/ 下符合规则的图片、JS、CSS、音频、字体、WebAssembly 和静态 JSON。这些域名下的其它请求不缓存。\n\n以下两个主站域名的 HTTPS 443 端口请求会转发并进行连接优化，同时用于识别资源和场景；响应不缓存：\n\n• game.granbluefantasy.jp\n• gbf.game.mbga.jp\n\n主站请求可以绕过本应用，但可能降低资源预载效果。\n\n其它域名或端口的请求无需进入本应用；若进入，则仅转发，不解析、不缓存。其中 HTTPS 保持加密传输，例如 ws.game.granbluefantasy.jp:11240。",
  ),
  (
    "缓存统计",
    "前三项仅统计上述 CDN 缓存范围内的资源：\n\n• 预载完成：本轮在浏览器请求前，后台提前下载并保存的资源数，不含已有缓存的读取。\n• 资源回源：本应用向 CDN 服务器发起资源请求的次数，包含后台预载。\n• 缓存命中：浏览器发来的资源请求，由本应用内存或磁盘缓存直接响应的次数。\n• 网络异常：本应用处理的 CDN 及两个主站域名请求发生网络错误或收到 HTTP 4xx/5xx 的次数，包含后台请求，但不计入后台预载返回的 404。\n\n处理范围之外的域名请求，以及浏览器自身缓存直接响应的请求，不计入统计。\n\n每次启动缓存服务或清理缓存时，四项计数归零。停止服务后保留本轮计数。各项可能重叠，不能直接相加。",
  ),
  (
    "日常使用",
    "保留浏览器缓存即可。实时预载会提前下载可能用到的资源，也可能增加流量。\n\n内存缓存默认 128 MiB，可调整，0 表示关闭；磁盘缓存不设容量上限。服务启动后，从磁盘恢复上次的内存资源，最多占内存上限的一半，不产生额外下载。连续 5 分钟未被实际请求的资源会退出内存，磁盘缓存保留；内存不足时会提前淘汰低热度资源。",
  ),
  (
    "设置与连接",
    "连接和资源缓存设置自动保存，无需手动确认；输入有误或保存失败时会提示。\n\n左侧齿轮可设置：\n\n• 主题：跟随系统、亮色或暗色。\n• 开机启动：默认关闭。\n• 启动后自动开启缓存服务：默认关闭。\n• 启动时隐藏到托盘：默认关闭。\n• 关闭窗口时隐藏到托盘：默认开启。\n\n主界面的“局域网连接”默认关闭。开启后接受局域网设备连接；运行时切换会重启服务，中断当前连接。\n\n连接设置中的“检测连接”使用当前填写的配置访问 https://game.granbluefantasy.jp/。输入离开编辑框后自动保存，关闭设置时也会校验并保存。检测结果只表示此时链路是否可用。",
  ),
  (
    "CA 证书",
    "证书管理可检查当前 CA 是否有效及 Windows 是否信任，并提供安装、卸载和重新生成功能。\n\n重复安装保留同一张 CA。卸载只移除当前用户对该证书的信任，不删除本地证书；本地计算机级信任需以管理员权限处理。\n\n重新生成需要确认并停止服务，之后需安装新证书。其它使用旧 CA 的设备也需重新安装。\n\ngbf-flash-cache.cer 是公开证书，可提供给需要连接本应用的设备。\n\n不要分享 data/ca/authority.json，其中包含 CA 私钥。泄漏后，他人可能伪造被已信任该 CA 的设备接受的证书。",
  ),
  (
    "数据",
    "配置和 CA 保存在程序旁的 data 文件夹；缓存和日志默认也在其中。\n\n停止服务后可更改缓存目录和日志目录。所选目录下会创建应用专用子目录。迁移缓存会复制资源并保留旧目录；使用新目录不会复制旧缓存。\n\n上游代理密码使用当前 Windows 账号加密保存。日志默认在下次启动应用时清理；需要保留时勾选“保留日志”，停止服务后可导出。\n\n更新前退出旧版，保留完整 data 文件夹及自行配置的缓存、日志目录。",
  ),
  ("关闭应用", "如未设置合适的分流规则，关闭应用前，请取消浏览器指向本应用的代理。"),
];

const mobileFeatures = <(String, String)>[
  (
    "缓存范围",
    "仅缓存指定 GBF CDN 域名 HTTPS 443 端口下 /assets/、/assets_en/ 内符合规则的静态资源：图片、JS、CSS、音频、字体、WebAssembly 和静态 JSON。\n\nCDN 域名：\nprd-game-a-gbf.akamaized.net 及 a1～a5 分片；prd-game-a-granbluefantasy.akamaized.net 及 a1～a5 分片；prd-game-a-granbluefantasy-steam.akamaized.net 及 a1～a5 分片；granbluefantasy.akamaized.net、gbf.akamaized.net。\n\ngame.granbluefantasy.jp、gbf.game.mbga.jp 的 HTTPS 443 请求会转发并进行连接优化，用于识别资源和场景，响应不缓存。绕过本应用可能降低资源预载效果。\n\n其它域名或端口无需进入本应用；若进入，仅转发，不解析、不缓存，其中 HTTPS 保持加密传输。",
  ),
  (
    "设置与连接",
    "连接和资源缓存设置自动保存，无需手动确认；输入有误或保存失败时会提示。\n\n“检测连接”使用当前填写的配置访问 https://game.granbluefantasy.jp/，结果只表示此时链路是否可用。",
  ),
  (
    "局域网连接",
    "开启首页的“局域网连接”后，其它设备填写运行本应用手机的局域网 IP 和监听端口，并安装相同的 CA 证书。运行时切换会重启服务，中断当前连接。",
  ),
  (
    "缓存统计",
    "前三项仅统计上述 CDN 缓存范围内的资源。\n预载完成：本轮在浏览器请求前，后台提前下载并保存的资源数。\n资源回源：本应用向 CDN 服务器发起资源请求的次数，包含后台预载。\n缓存命中：浏览器资源请求由本应用内存或磁盘缓存直接响应的次数。\n网络异常：本应用处理的 CDN 及两个主站域名请求发生网络错误或收到 HTTP 4xx/5xx 的次数，包含后台请求，但不计入后台预载返回的 404。\n\n范围外及浏览器自身缓存响应的请求不计入统计。启动缓存服务或清理缓存时计数归零，停止后保留本轮计数。各项可能重叠，不能直接相加。",
  ),
  (
    "缓存与数据",
    "保留浏览器缓存即可。实时预载可能增加流量。内存默认 128 MiB，0 为关闭；磁盘不设容量上限。启动时最多恢复内存上限的一半，不额外下载。连续 5 分钟未被实际请求的资源会退出内存，磁盘缓存保留；内存不足时优先淘汰低热度资源。\n\n数据保存在应用私有目录。上游密码由 Android 系统密钥加密保存。日志未开启保留时，下次启动会清理旧日志。停止服务后可清理缓存、导出日志。卸载应用会删除应用数据，包含缓存、配置及 CA 私钥；更新请直接覆盖安装。",
  ),
  (
    "CA 证书",
    "在“CA 证书”页导出证书，通过系统安全设置安装，并确认浏览器信任它。Firefox 使用独立证书库时，需导入或启用对系统用户证书的信任。\n\n重复导出使用同一张 CA。重新生成后需在浏览器和其它设备上重新安装。可通过系统证书设置移除旧证书。公开的 .cer 文件可分享给需要连接的设备，切勿分享包含 CA 私钥的应用数据。",
  ),
  (
    "后台与停止",
    "离开界面后，已启动的缓存服务继续运行，可在应用或通知中停止。系统可能限制后台运行；仅关闭界面不等于停止服务。如未设置合适的分流规则，停止前请取消浏览器指向本应用的代理。",
  ),
];

const verification = '''正常打开几个游戏页面，观察资源回源、预载完成和缓存命中计数。浏览器自身缓存命中时，请求不会进入 GFC，计数不一定增加，无需为此清空浏览器缓存。

“运行中”只表示服务已启动；“检测连接”成功只表示此时 GFC 出站可用，不能单独证明浏览器已接入。

切换网络或加速器节点后，若连接持续异常，可尝试在应用内停止并重新启动服务，或完全退出应用后重新打开。''';
const windowsAccelerator = '''在“设置”中开启，重启后以 chrome.exe 进程名运行，供按进程名识别的游戏加速器接管流量。默认关闭，关闭后重启恢复。

仍从 GBF Flash Cache.exe 启动，无需手动改名。两种模式共用配置、缓存和证书。''';
const androidProxy =
    '''开启此选项后，点击“启动”开始接管，点击“停止”结束接管。TCP／UDP 出站行为与上表的 SOCKS5 入站一致。

此功能通过 Android VpnService 实现，不能与其它使用 VpnService 的代理或加速器同时运行。

• 全部应用：包含、排除名单均不生效。
• 包含应用：仅接管包含名单中的应用。
• 排除应用：接管排除名单以外的应用。

两份名单独立保存，本应用始终绕过自身接管。

使用安全 DNS 或 Android 私人 DNS 时，流量可能无法按域名进入缓存。其它应用若不信任用户安装的 CA，也不能直接使用缓存范围内的 HTTPS 资源。''';
const localLoop = '''使用本地上游代理时，上游应用应仅提供代理端口，不启用 VpnService。

必须让上游代理应用绕过 GFC 的系统代理，避免上游出站流量再次进入本应用形成回环：
• 使用“包含应用”时，不包含上游代理应用。
• 使用“排除应用”时，将上游代理应用排除。
• 不要选择“全部应用”。

错误配置可能导致网络无法访问或设备卡顿。请停止服务，调整应用名单后再启动。''';

class HelpGuide extends StatefulWidget {
  const HelpGuide({super.key, required this.mobile});
  final bool mobile;
  @override
  State<HelpGuide> createState() => _HelpGuideState();
}

class _HelpGuideState extends State<HelpGuide> {
  bool features = false;
  Widget paragraph(String text) => SelectableText(
    text,
    key: PageStorageKey(text),
    style: const TextStyle(height: 1.7),
  );
  Widget section(String title, Widget content, {bool open = false}) => Padding(
    padding: const EdgeInsets.only(bottom: 16),
    child: Card(
      margin: EdgeInsets.zero,
      elevation: 0,
      color: Theme.of(context).colorScheme.surface,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(20)),
      clipBehavior: Clip.antiAlias,
      child: ExpansionTile(
        key: PageStorageKey('${features ? 'features' : 'start'}:$title'),
        initiallyExpanded: open,
        maintainState: true,
        tilePadding: const EdgeInsets.symmetric(horizontal: 24, vertical: 8),
        childrenPadding: const EdgeInsets.fromLTRB(24, 0, 24, 24),
        expandedCrossAxisAlignment: CrossAxisAlignment.start,
        shape: const Border(),
        collapsedShape: const Border(),
        title: Text(title, style: Theme.of(context).textTheme.titleMedium),
        children: [SizedBox(width: double.infinity, child: content)],
      ),
    ),
  );
  Widget steps(List<String> values, String note) => Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      for (var i = 0; i < values.length; i++)
        Padding(
          padding: const EdgeInsets.only(bottom: 16),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Container(
                width: 28,
                height: 28,
                alignment: Alignment.center,
                decoration: BoxDecoration(
                  color: Theme.of(context).colorScheme.secondaryContainer,
                  borderRadius: BorderRadius.circular(8),
                ),
                child: Text(
                  '${i + 1}',
                  style: TextStyle(
                    color: Theme.of(context).colorScheme.primary,
                  ),
                ),
              ),
              const SizedBox(width: 12),
              Expanded(child: paragraph(values[i])),
            ],
          ),
        ),
      const Divider(),
      const SizedBox(height: 12),
      paragraph(note),
    ],
  );
  List<Widget> starting() => [
    Padding(
      padding: const EdgeInsets.only(bottom: 20),
      child: paragraph('以下以监听端口 8765 为例，修改过端口时请填写实际值。'),
    ),
    section(
      widget.mobile ? '浏览器接入' : '通过浏览器使用',
      steps(
        [
          widget.mobile
              ? '导出并安装 CA 证书，确认浏览器信任它。'
              : '在“管理证书”中安装 CA 证书。Firefox 使用独立证书库时，还需导入该证书。',
          widget.mobile
              ? '保持 GFC 的“系统代理”关闭。如需指定代理端口，再开启“上游代理”，填写协议、地址和端口。'
              : '如需通过其它代理出站，开启“上游代理”，填写协议、地址和端口；否则保持关闭。',
          '启动缓存服务，确认显示“运行中”。',
          '在浏览器代理设置或插件中，选择 HTTP，地址填写 127.0.0.1，端口填写 8765，然后打开游戏。',
        ],
        widget.mobile
            ? '适用于能够配置代理的浏览器，可配合已有加速器或代理应用。已有加速器或代理按应用分流时，请确保其处理范围包含 GFC。'
            : 'GFC 不会自动修改 Windows 系统代理。可将全部浏览器流量发给 GFC，也可自行配置分流规则。',
      ),
      open: true,
    ),
    if (widget.mobile)
      for (final include in [true, false])
        section(
          '系统代理 · ${include ? '包含应用' : '排除应用'}',
          steps(
            [
              '导出并安装 CA 证书，确认浏览器信任它。',
              include
                  ? '开启“系统代理”，应用范围选择“包含应用”，选中需要使用的浏览器。'
                  : '开启“系统代理”，应用范围选择“排除应用”，选中不需要经过 GFC 的应用。',
              '如需上游代理，开启“上游代理”并填写协议、地址和端口；否则保持关闭。',
              '点击“启动”，首次使用时确认 Android 的连接授权。',
              include ? '在所选浏览器中打开游戏，无需配置浏览器代理。' : '在未被排除的浏览器中打开游戏，无需配置浏览器代理。',
            ],
            '${include ? '只有包含名单中的应用经过 GFC。' : '除排除名单及 GFC 自身外，其它应用均经过 GFC。'}使用本地上游时，上游应用仅提供代理端口，不启用 VpnService，${include ? '也不要将它加入包含名单' : '并将它加入排除名单'}，避免其出站流量再次进入 GFC 形成回环。',
          ),
        ),
    section('连接检查与排查', paragraph(verification)),
  ];
  Widget forwarding() {
    const rows = [
      ['入站协议', '上游协议', 'TCP 出站', 'UDP 出站'],
      ['HTTP／HTTPS／SOCKS4', '无', '直接出站', '入站不支持'],
      ['HTTP／HTTPS／SOCKS4', 'HTTP／HTTPS／SOCKS4／SOCKS5', '经上游', '入站不支持'],
      ['SOCKS5', '无', '直接出站', '直接出站'],
      ['SOCKS5', 'HTTP／HTTPS／SOCKS4', '经上游', '直接出站'],
      ['SOCKS5', 'SOCKS5', '经上游', '经上游'],
    ];
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Table(
          columnWidths: const {
            0: FlexColumnWidth(1.35),
            1: FlexColumnWidth(1.35),
            2: FlexColumnWidth(),
            3: FlexColumnWidth(),
          },
          border: TableBorder.all(color: Theme.of(context).dividerColor),
          defaultVerticalAlignment: TableCellVerticalAlignment.middle,
          children: [
            for (var i = 0; i < rows.length; i++)
              TableRow(
                children: [
                  for (final value in rows[i])
                    Padding(
                      padding: const EdgeInsets.all(8),
                      child: Text(
                        value,
                        style: TextStyle(
                          fontSize: 13,
                          height: 1.5,
                          fontWeight: i == 0
                              ? FontWeight.w600
                              : FontWeight.normal,
                        ),
                      ),
                    ),
                ],
              ),
          ],
        ),
        const SizedBox(height: 16),
        paragraph('经上游转发失败时，不会自动改为直接出站。UDP 仅转发，不缓存。'),
      ],
    );
  }

  @override
  Widget build(BuildContext context) {
    final descriptions = widget.mobile ? mobileFeatures : desktopFeatures;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        if (!widget.mobile) ...[
          Text('使用说明', style: Theme.of(context).textTheme.headlineSmall),
          const SizedBox(height: 20),
        ],
        DefaultTabController(
          length: 2,
          child: TabBar(
            onTap: (index) => setState(() => features = index == 1),
            tabs: const [
              Tab(text: '开始使用'),
              Tab(text: '功能说明'),
            ],
          ),
        ),
        const SizedBox(height: 24),
        if (!features)
          ...starting()
        else ...[
          for (var i = 0; i < descriptions.length; i++) ...[
            section(
              descriptions[i].$1,
              paragraph(descriptions[i].$2),
              open: i == 0,
            ),
            if (descriptions[i].$1 == '设置与连接')
              section('TCP／UDP 转发', forwarding()),
          ],
          if (!widget.mobile) ...[
            const Divider(height: 40),
            Text('Windows 版本特性', style: Theme.of(context).textTheme.titleLarge),
            const SizedBox(height: 20),
            section('加速器兼容模式', paragraph(windowsAccelerator)),
          ],
          if (widget.mobile) ...[
            const Divider(height: 40),
            Text('Android 版本特性', style: Theme.of(context).textTheme.titleLarge),
            const SizedBox(height: 20),
            section('系统代理', paragraph(androidProxy)),
            section('本地上游防回环', paragraph(localLoop)),
            section(
              '后台运行',
              paragraph(
                '''GFC 需要在使用浏览器期间保持后台运行。部分手机的省电或后台管理可能限制网络连接或停止服务，表现为切换到浏览器后加载缓慢、超时或无法访问。

遇到这类问题，请在系统应用设置中允许 GFC 后台活动，必要时将电池策略设为“不限制”或取消电池优化，具体名称因系统而异。使用本机上游代理时，也需检查该代理应用的后台设置；任一方受限都可能中断连接。''',
              ),
            ),
          ],
        ],
      ],
    );
  }
}
