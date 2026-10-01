using System.Buffers.Binary;
using System.Text;
using System.Xml.Linq;

namespace QuickADB.Core.Installation;

internal static class AndroidManifest
{
    public static Dictionary<string, string> ReadAttributes(byte[] data)
    {
        if (data.Length == 0) throw new InvalidDataException("APK 清单为空。");
        if (data[0] == '<' || data[0] == 0xEF)
        {
            using var stream = new MemoryStream(data);
            var root = XDocument.Load(stream).Root ?? throw new InvalidDataException("APK 清单为空。");
            return root.Attributes().Where(x => !x.IsNamespaceDeclaration).ToDictionary(x => x.Name.LocalName, x => x.Value);
        }
        ushort U16(int position) => position >= 0 && position + 2 <= data.Length ? BinaryPrimitives.ReadUInt16LittleEndian(data.AsSpan(position, 2)) : throw new InvalidDataException("APK 清单已损坏。");
        uint U32(int position) => position >= 0 && position + 4 <= data.Length ? BinaryPrimitives.ReadUInt32LittleEndian(data.AsSpan(position, 4)) : throw new InvalidDataException("APK 清单已损坏。");
        if (U16(0) != 3 || U32(4) != data.Length) throw new InvalidDataException("APK 二进制清单格式无效。");
        var strings = new List<string>();
        string StringAt(uint index) => index < strings.Count ? strings[(int)index] : throw new InvalidDataException("APK 清单字符串索引无效。");
        for (var offset = (int)U16(2); offset < data.Length;)
        {
            var type = U16(offset);
            var headerSize = U16(offset + 2);
            var size = checked((int)U32(offset + 4));
            if (size < headerSize || size < 8 || offset + (long)size > data.Length) throw new InvalidDataException("APK 清单数据块无效。");
            if (type == 1)
            {
                var count = checked((int)U32(offset + 8));
                var utf8 = (U32(offset + 16) & 0x100) != 0;
                var start = checked((int)U32(offset + 20));
                if (count < 0 || headerSize + (long)count * 4 > size) throw new InvalidDataException("APK 清单字符串池无效。");
                for (var index = 0; index < count; index++)
                {
                    var position = checked(offset + start + (int)U32(offset + headerSize + index * 4));
                    int Length8()
                    {
                        if (position >= offset + size) throw new InvalidDataException("APK 清单字符串无效。");
                        var length = data[position++];
                        if ((length & 0x80) == 0) return length;
                        if (position >= offset + size) throw new InvalidDataException("APK 清单字符串无效。");
                        return ((length & 0x7F) << 8) | data[position++];
                    }
                    int length;
                    if (utf8)
                    {
                        Length8();
                        length = Length8();
                    }
                    else
                    {
                        length = U16(position);
                        position += 2;
                        if ((length & 0x8000) != 0)
                        {
                            length = ((length & 0x7FFF) << 16) | U16(position);
                            position += 2;
                        }
                        length = checked(length * 2);
                    }
                    if (position < offset || position + (long)length > offset + size) throw new InvalidDataException("APK 清单字符串超出范围。");
                    strings.Add((utf8 ? Encoding.UTF8 : Encoding.Unicode).GetString(data, position, length));
                }
            }
            else if (type == 0x102 && StringAt(U32(offset + 20)) == "manifest")
            {
                var attributes = new Dictionary<string, string>(StringComparer.Ordinal);
                var attributeStart = U16(offset + 24);
                var attributeSize = U16(offset + 26);
                var attributeCount = U16(offset + 28);
                if (attributeSize < 20 || 16L + attributeStart + (long)attributeSize * attributeCount > size)
                    throw new InvalidDataException("APK 清单属性表无效。");
                for (var index = 0; index < attributeCount; index++)
                {
                    var attribute = offset + 16 + attributeStart + index * attributeSize;
                    var name = StringAt(U32(attribute + 4));
                    var raw = U32(attribute + 8);
                    var valueType = data[attribute + 15];
                    var value = U32(attribute + 16);
                    attributes[name] = raw != uint.MaxValue ? StringAt(raw) : valueType == 3 ? StringAt(value) : value.ToString(System.Globalization.CultureInfo.InvariantCulture);
                }
                return attributes;
            }
            offset += size;
        }
        throw new InvalidDataException("APK 清单缺少 manifest 节点。");
    }
}
