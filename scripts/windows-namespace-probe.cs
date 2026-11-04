// Standalone public-byte compatibility experiment; never a production storage backend.
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

public static class WindowsNamespaceProbe
{
    const uint Read = 0x80000000, Write = 0x40000000, Delete = 0x10000;
    const uint ReadControl = 0x20000, ShareReadWrite = 3;
    const uint BackupSemantics = 0x02000000, OpenReparse = 0x00200000;
    const uint RestrictedFlags = 5, Enabled = 4, DenyOnly = 16, Integrity = 32;
    const string Admin = "S-1-5-32-544";
    static readonly byte[] PublicBytes = Encoding.UTF8.GetBytes("namespace-probe-public\n");
    static readonly HashSet<string> OrdinaryGroups = new HashSet<string> {
        "S-1-1-0", "S-1-2-0", "S-1-2-1", "S-1-5-4", "S-1-5-11", "S-1-5-15", "S-1-5-32-545"
    };
    [StructLayout(LayoutKind.Sequential)] struct SidAttributes { public IntPtr Sid; public uint Attributes; }
    [StructLayout(LayoutKind.Sequential)] struct GroupsHeader { public uint Count; public SidAttributes First; }
    [StructLayout(LayoutKind.Sequential)] struct Luid { public uint Low; public int High; }
    [StructLayout(LayoutKind.Sequential)] struct Privilege { public Luid Id; public uint Attributes; }
    [StructLayout(LayoutKind.Sequential)] struct PrivilegesHeader { public uint Count; public Privilege First; }
    [StructLayout(LayoutKind.Sequential)] struct SecurityAttributes { public uint Length; public IntPtr Descriptor; public int Inherit; }
    [StructLayout(LayoutKind.Sequential)] struct UnicodeString { public ushort Length, MaximumLength; public IntPtr Buffer; }
    [StructLayout(LayoutKind.Sequential)] struct ObjectAttributes {
        public uint Length; public IntPtr RootDirectory, ObjectName; public uint Attributes;
        public IntPtr SecurityDescriptor, SecurityQualityOfService;
    }
    [StructLayout(LayoutKind.Sequential)] struct RenameHeader { public uint Replace; public IntPtr Root; public uint NameLength; public ushort FirstCharacter; }
    [DllImport("kernel32.dll")] static extern IntPtr GetCurrentProcess();
    [DllImport("kernel32.dll")] static extern IntPtr GetCurrentThread();
    [DllImport("kernel32.dll", SetLastError=true)] static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);
    [DllImport("advapi32.dll", SetLastError=true)] static extern bool OpenProcessToken(IntPtr process, uint rights, out SafeFileHandle token);
    [DllImport("advapi32.dll", SetLastError=true)] static extern bool OpenThreadToken(IntPtr thread, uint rights, bool asSelf, out SafeFileHandle token);
    [DllImport("advapi32.dll", SetLastError=true)] static extern bool GetTokenInformation(SafeFileHandle token, int kind, IntPtr buffer, uint size, out uint needed);
    [DllImport("advapi32.dll", SetLastError=true)] static extern bool SetTokenInformation(SafeFileHandle token, int kind, IntPtr buffer, uint size);
    [DllImport("advapi32.dll", SetLastError=true)] static extern bool CreateRestrictedToken(SafeFileHandle source, uint flags, uint disabledCount, IntPtr disabled, uint deleteCount, IntPtr deleted, uint restrictCount, IntPtr restricted, out SafeFileHandle token);
    [DllImport("advapi32.dll", SetLastError=true)] static extern bool ImpersonateLoggedOnUser(SafeFileHandle token);
    [DllImport("advapi32.dll", SetLastError=true)] static extern bool RevertToSelf();
    [DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern bool ConvertSidToStringSidW(IntPtr sid, out IntPtr text);
    [DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern bool ConvertStringSidToSidW(string text, out IntPtr sid);
    [DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern bool ConvertStringSecurityDescriptorToSecurityDescriptorW(string text, uint revision, out IntPtr descriptor, out uint size);
    [DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern bool LookupPrivilegeValueW(string system, string name, out Luid value);
    [DllImport("advapi32.dll")] static extern uint GetLengthSid(IntPtr sid);
    [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr memory);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern bool CreateDirectoryW(string path, ref SecurityAttributes attributes);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern SafeFileHandle CreateFileW(string path, uint rights, uint sharing, IntPtr attributes, uint creation, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool WriteFile(SafeFileHandle file, [In] byte[] bytes, uint count, out uint written, IntPtr overlapped);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool ReadFile(SafeFileHandle file, [Out] byte[] bytes, uint count, out uint read, IntPtr overlapped);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool SetFilePointerEx(SafeFileHandle file, long offset, out long position, uint origin);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool FlushFileBuffers(SafeFileHandle file);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool GetFileInformationByHandleEx(SafeFileHandle file, int kind, IntPtr buffer, uint size);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool SetFileInformationByHandle(SafeFileHandle file, int kind, IntPtr buffer, uint size);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern bool GetVolumeInformationByHandleW(SafeFileHandle file, StringBuilder volume, uint volumeSize, out uint serial, out uint maxName, out uint flags, StringBuilder system, uint systemSize);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern uint GetFinalPathNameByHandleW(SafeFileHandle file, StringBuilder name, uint size, uint flags);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode)] static extern uint GetDriveTypeW(string root);
    [DllImport("ntdll.dll")] static extern int NtCreateFile(out IntPtr file, uint rights, ref ObjectAttributes attributes, IntPtr status, IntPtr allocation, uint fileAttributes, uint sharing, uint disposition, uint options, IntPtr ea, uint eaLength);
    [DllImport("ntdll.dll")] static extern int NtFlushBuffersFileEx(SafeFileHandle file, uint flags, IntPtr parameters, uint size, IntPtr status);

    sealed class Buffer : IDisposable {
        public IntPtr Pointer; public int Length;
        public Buffer(int length) { Length = length; Pointer = Marshal.AllocHGlobal(length); Marshal.Copy(new byte[length], 0, Pointer, length); }
        public void Dispose() { if (Pointer != IntPtr.Zero) { Marshal.FreeHGlobal(Pointer); Pointer = IntPtr.Zero; } }
    }
    sealed class Descriptor : IDisposable {
        public IntPtr Pointer;
        public Descriptor(string owner, bool directory, bool adminOnly=false) {
            string ace = adminOnly ? "BA" : owner;
            Require(ConvertStringSecurityDescriptorToSecurityDescriptorW("O:" + owner + "D:P(A;" + (directory ? "OICI" : "") + ";FA;;;" + ace + ")", 1, out Pointer, out _), "security_descriptor");
        }
        public void Dispose() { if (Pointer != IntPtr.Zero) { LocalFree(Pointer); Pointer = IntPtr.Zero; } }
    }
    sealed class NativeFailure : Exception {
        public readonly string Stage; public readonly int Code;
        public NativeFailure(string stage, int code) : base(stage) { Stage=stage; Code=code; }
    }
    static void Require(bool ok, string stage) { if (!ok) throw new NativeFailure(stage, Marshal.GetLastWin32Error()); }
    static void Check(bool ok, string stage) { if (!ok) throw new NativeFailure(stage, 0); }
    static void Abort(string stage,uint code) {
        // The parent can publish this fixed-safe packet even when an ownership
        // failure requires immediate isolated-process termination, before Run returns.
        try {
            Console.Out.WriteLine("{\"schema\":1,\"complete\":false,\"candidates\":[],\"failed_stage\":\""+stage+"\",\"error_code\":"+code+"}");
            Console.Out.Flush();
        } finally { Environment.FailFast("Native namespace probe could not safely finish its isolated ownership scope"); }
    }
    static Dictionary<string, object> Row(params object[] fields) {
        var row = new Dictionary<string, object>();
        for (int i=0; i<fields.Length; i+=2) row.Add((string)fields[i], fields[i+1]);
        return row;
    }
    static Buffer TokenInfo(SafeFileHandle token, int kind) {
        GetTokenInformation(token, kind, IntPtr.Zero, 0, out uint size);
        if (size==0) throw new NativeFailure("token_information_size",Marshal.GetLastWin32Error());
        var buffer = new Buffer(checked((int)size));
        if (!GetTokenInformation(token, kind, buffer.Pointer, size, out size)) { int error=Marshal.GetLastWin32Error(); buffer.Dispose(); throw new NativeFailure("token_information", error); }
        return buffer;
    }
    static string SidText(IntPtr sid) {
        Require(ConvertSidToStringSidW(sid, out IntPtr text), "sid_text");
        try { return Marshal.PtrToStringUni(text); } finally { LocalFree(text); }
    }
    static string User(SafeFileHandle token) { using (var b=TokenInfo(token, 1)) return SidText(Marshal.ReadIntPtr(b.Pointer)); }
    static string IntegritySid(SafeFileHandle token) { using (var b=TokenInfo(token, 25)) return SidText(Marshal.ReadIntPtr(b.Pointer)); }
    static uint IntegrityRid(SafeFileHandle token) { return uint.Parse(IntegritySid(token).Split('-').Last()); }
    static uint TokenNumber(SafeFileHandle token, int kind) { using (var b=TokenInfo(token, kind)) return unchecked((uint)Marshal.ReadInt32(b.Pointer)); }
    static List<SidAttributes> Groups(Buffer buffer) {
        uint count=unchecked((uint)Marshal.ReadInt32(buffer.Pointer));
        int offset=Marshal.OffsetOf<GroupsHeader>("First").ToInt32(), stride=Marshal.SizeOf<SidAttributes>();
        Check(checked(offset + (long)count*stride) <= buffer.Length, "token_groups_extent");
        var groups=new List<SidAttributes>();
        for (uint i=0; i<count; i++) groups.Add(Marshal.PtrToStructure<SidAttributes>(IntPtr.Add(buffer.Pointer, checked(offset+(int)i*stride))));
        return groups;
    }
    static bool Ordinary(SidAttributes group) { return (group.Attributes & Integrity) != 0 || (group.Attributes & 0xc0000000) == 0xc0000000 || OrdinaryGroups.Contains(SidText(group.Sid)); }
    static Dictionary<string, object> TokenReport(SafeFileHandle token, bool proveOrdinary) {
        using (var groups=TokenInfo(token, 2)) using (var privileges=TokenInfo(token, 3)) {
            var groupRows=new List<object>();
            foreach (var group in Groups(groups)) {
                string sid=SidText(group.Sid);
                if (proveOrdinary) Check((group.Attributes & Enabled) == 0 || Ordinary(group), "unexpected_enabled_group");
                if (proveOrdinary && sid == Admin) Check((group.Attributes & DenyOnly) != 0 && (group.Attributes & Enabled) == 0, "administrators_not_deny_only");
                groupRows.Add(Row("sid",sid,"attributes",group.Attributes));
            }
            Require(LookupPrivilegeValueW(null,"SeChangeNotifyPrivilege",out Luid notify),"notify_privilege_identity");
            int offset=Marshal.OffsetOf<PrivilegesHeader>("First").ToInt32(), stride=Marshal.SizeOf<Privilege>();
            uint count=unchecked((uint)Marshal.ReadInt32(privileges.Pointer));
            Check(checked(offset+(long)count*stride)<=privileges.Length,"token_privileges_extent");
            var privilegeRows=new List<object>();
            for (uint i=0; i<count; i++) {
                var privilege=Marshal.PtrToStructure<Privilege>(IntPtr.Add(privileges.Pointer,checked(offset+(int)i*stride)));
                if (proveOrdinary && (privilege.Attributes & 2) != 0) Check(privilege.Id.Low==notify.Low && privilege.Id.High==notify.High,"unexpected_enabled_privilege");
                privilegeRows.Add(Row("luid_low",privilege.Id.Low,"luid_high",privilege.Id.High,"attributes",privilege.Attributes));
            }
            if (proveOrdinary) { Check(TokenNumber(token,21)!=0,"token_has_no_restrictions"); Check(IntegrityRid(token)<=8192,"integrity_above_medium"); }
            return Row("user_sid",User(token),"has_restrictions",TokenNumber(token,21),"elevated",TokenNumber(token,20),"integrity_sid",IntegritySid(token),"groups",groupRows,"privileges",privilegeRows);
        }
    }
    static SafeFileHandle Restricted(SafeFileHandle source) {
        using (var originalGroups=TokenInfo(source,2)) {
            var disable=Groups(originalGroups).Where(group=>!Ordinary(group)).ToList();
            int stride=Marshal.SizeOf<SidAttributes>();
            using (var disabled=new Buffer(checked(disable.Count*stride))) {
                for (int i=0;i<disable.Count;i++) Marshal.StructureToPtr(disable[i],IntPtr.Add(disabled.Pointer,i*stride),false);
                Require(CreateRestrictedToken(source,RestrictedFlags,(uint)disable.Count,disabled.Pointer,0,IntPtr.Zero,0,IntPtr.Zero,out SafeFileHandle token),"create_restricted_token");
                try {
                    uint target=Math.Min(8192u,IntegrityRid(source));
                    if (IntegrityRid(token)>target) {
                        Require(ConvertStringSidToSidW("S-1-16-"+target,out IntPtr sid),"integrity_sid_create");
                        try { using (var label=new Buffer(checked(Marshal.SizeOf<SidAttributes>()+(int)GetLengthSid(sid)))) { Marshal.StructureToPtr(new SidAttributes{Sid=sid,Attributes=Integrity},label.Pointer,false); Require(SetTokenInformation(token,25,label.Pointer,(uint)label.Length),"lower_new_token_integrity"); } }
                        finally { LocalFree(sid); }
                    }
                    return token;
                } catch { token.Dispose(); throw; }
            }
        }
    }
    static SecurityAttributes Attributes(Descriptor descriptor) { return new SecurityAttributes { Length=(uint)Marshal.SizeOf<SecurityAttributes>(),Descriptor=descriptor.Pointer,Inherit=0 }; }
    static void CreateDirectory(string path,string sid) { using (var d=new Descriptor(sid,true)) { var sa=Attributes(d); Require(CreateDirectoryW(path,ref sa),"create_private_directory"); } }
    static SafeFileHandle CreatePublicFile(string path,string sid,bool adminOnly=false) {
        using (var d=new Descriptor(sid,false,adminOnly)) using (var a=new Buffer(Marshal.SizeOf<SecurityAttributes>())) {
            Marshal.StructureToPtr(Attributes(d),a.Pointer,false);
            var file=CreateFileW(path,Read|Write|Delete|ReadControl,ShareReadWrite,a.Pointer,1,OpenReparse,IntPtr.Zero);
            if (file.IsInvalid) { int code=Marshal.GetLastWin32Error(); file.Dispose(); throw new NativeFailure("create_public_file",code); }
            return file;
        }
    }
    static SafeFileHandle OpenWin32(string path,uint rights,uint flags,uint sharing=ShareReadWrite) {
        var file=CreateFileW(path,rights,sharing,IntPtr.Zero,3,flags,IntPtr.Zero);
        if (file.IsInvalid) { int code=Marshal.GetLastWin32Error(); file.Dispose(); throw new NativeFailure("open_win32",code); }
        return file;
    }
    static void WritePublic(SafeFileHandle file) { Require(WriteFile(file,PublicBytes,(uint)PublicBytes.Length,out uint written,IntPtr.Zero),"write_public_bytes"); Check(written==PublicBytes.Length,"short_public_write"); }
    static void ReadPublic(SafeFileHandle file) {
        Require(SetFilePointerEx(file,0,out _,0),"seek_public_bytes");
        var bytes=new byte[PublicBytes.Length+1];
        Require(ReadFile(file,bytes,(uint)bytes.Length,out uint read,IntPtr.Zero),"read_public_bytes");
        Check(read==PublicBytes.Length && bytes.Take((int)read).SequenceEqual(PublicBytes),"public_bytes_changed");
    }
    static Dictionary<string, object> Identity(SafeFileHandle file,bool directory) {
        using (var id=new Buffer(24)) using (var standard=new Buffer(24)) using (var tag=new Buffer(8)) {
            Require(GetFileInformationByHandleEx(file,18,id.Pointer,(uint)id.Length),"file_identity");
            Require(GetFileInformationByHandleEx(file,1,standard.Pointer,(uint)standard.Length),"file_standard_information");
            Require(GetFileInformationByHandleEx(file,9,tag.Pointer,(uint)tag.Length),"file_attribute_tag");
            uint attributes=unchecked((uint)Marshal.ReadInt32(tag.Pointer));
            uint links=unchecked((uint)Marshal.ReadInt32(standard.Pointer,16));
            Check((attributes&0x400)==0,"reparse_object");
            Check(((attributes&0x10)!=0)==directory,"wrong_object_kind");
            if (!directory) Check(links==1,"linked_regular_file");
            var fileId=new byte[16]; Marshal.Copy(IntPtr.Add(id.Pointer,8),fileId,0,fileId.Length);
            return Row("volume_id",unchecked((ulong)Marshal.ReadInt64(id.Pointer)),"file_id",BitConverter.ToString(fileId).Replace("-",""),"links",links,"attributes",attributes);
        }
    }
    static void SameIdentity(Dictionary<string,object> a,Dictionary<string,object> b) { Check(a["volume_id"].Equals(b["volume_id"])&&a["file_id"].Equals(b["file_id"]),"file_identity_changed"); }
    static Dictionary<string,object> LocalNtfs(SafeFileHandle anchor) {
        var fs=new StringBuilder(32); var label=new StringBuilder(261);
        Require(GetVolumeInformationByHandleW(anchor,label,(uint)label.Capacity,out uint serial,out _,out uint flags,fs,(uint)fs.Capacity),"volume_information");
        Check(fs.ToString()=="NTFS","filesystem_not_ntfs");
        uint size=GetFinalPathNameByHandleW(anchor,null,0,1); if (size==0) throw new NativeFailure("volume_guid_path_size",Marshal.GetLastWin32Error());
        var final=new StringBuilder(checked((int)size+1));
        uint actual=GetFinalPathNameByHandleW(anchor,final,(uint)final.Capacity,1); if (actual==0) throw new NativeFailure("volume_guid_path",Marshal.GetLastWin32Error()); Check(actual<final.Capacity,"volume_guid_path_extent");
        string name=final.ToString(); int end=name.IndexOf("}\\",StringComparison.Ordinal);
        Check(name.StartsWith(@"\\?\Volume{",StringComparison.OrdinalIgnoreCase)&&end>=0,"not_local_volume_guid");
        string volumeRoot=name.Substring(0,end+2); uint drive=GetDriveTypeW(volumeRoot);
        Check(drive==3,"not_fixed_local_volume");
        return Row("filesystem",fs.ToString(),"volume_serial",serial,"filesystem_flags",flags,"drive_type",drive,"volume_guid",volumeRoot);
    }
    // IO_STATUS_BLOCK stays unmanaged and alive until even an unexpected pending
    // operation actually finishes; no transient out-struct or abandoned native I/O.
    static int Complete(int status,SafeFileHandle handle,Buffer iosb) {
        if (status==0x103) {
            uint waited=WaitForSingleObject(handle.DangerousGetHandle(),0xffffffff);
            if (waited!=0) Abort("pending_io_not_drained",waited==0xffffffff?unchecked((uint)Marshal.GetLastWin32Error()):waited);
            status=Marshal.ReadInt32(iosb.Pointer);
        }
        if (Marshal.ReadInt32(iosb.Pointer)==0x103) Abort("native_completion_still_pending",0x103);
        return status;
    }
    static SafeFileHandle OpenNative(string name,SafeFileHandle parent,List<object> stages,string stage) {
        byte[] encoded=Encoding.Unicode.GetBytes(name+"\0");
        using (var text=new Buffer(encoded.Length)) using (var unicode=new Buffer(Marshal.SizeOf<UnicodeString>())) using (var iosb=new Buffer(16)) {
            Marshal.Copy(encoded,0,text.Pointer,encoded.Length);
            Marshal.StructureToPtr(new UnicodeString{Length=checked((ushort)(encoded.Length-2)),MaximumLength=checked((ushort)encoded.Length),Buffer=text.Pointer},unicode.Pointer,false);
            var oa=new ObjectAttributes{Length=(uint)Marshal.SizeOf<ObjectAttributes>(),RootDirectory=parent==null?IntPtr.Zero:parent.DangerousGetHandle(),ObjectName=unicode.Pointer,Attributes=0x40};
            int returnedStatus=NtCreateFile(out IntPtr raw,0x1200a7,ref oa,iosb.Pointer,IntPtr.Zero,0,ShareReadWrite,1,0x200021,IntPtr.Zero,0);
            int status=returnedStatus;
            using (var pendingHandle=new SafeFileHandle(raw,false)) {
                if (status==0x103&&pendingHandle.IsInvalid) Abort("pending_open_has_no_completion_owner",unchecked((uint)status));
                status=Complete(status,pendingHandle,iosb);
            }
            stages.Add(Row("stage",stage,"returned_ntstatus",unchecked((uint)returnedStatus),"ntstatus",unchecked((uint)status),"io_status",unchecked((uint)Marshal.ReadInt32(iosb.Pointer)),"io_information",unchecked((ulong)Marshal.ReadInt64(iosb.Pointer,8)),"access",0x1200a7u,"share",ShareReadWrite,"options",0x200021u));
            var file=new SafeFileHandle(raw,true);
            int ioStatus=Marshal.ReadInt32(iosb.Pointer);
            if (status<0||ioStatus<0||file.IsInvalid) { file.Dispose(); throw new NativeFailure(stage,status<0?status:ioStatus); }
            return file;
        }
    }
    static SafeFileHandle OpenDirectory(string path,SafeFileHandle parent,string leaf,bool native,List<object> stages,string stage) {
        if (native) return OpenNative(parent==null?@"\??\"+Path.GetFullPath(path):leaf,parent,stages,stage);
        var file=CreateFileW(path,Read|Write|ReadControl,ShareReadWrite,IntPtr.Zero,3,BackupSemantics|OpenReparse,IntPtr.Zero);
        int error=file.IsInvalid?Marshal.GetLastWin32Error():0;
        stages.Add(Row("stage",stage,"win32_error",error,"access",Read|Write|ReadControl,"share",ShareReadWrite,"options",BackupSemantics|OpenReparse));
        if (file.IsInvalid) { file.Dispose(); throw new NativeFailure(stage,error); }
        return file;
    }
    static void DirectoryFlush(SafeFileHandle directory,bool native,List<object> stages,string stage) {
        if (native) {
            using (var iosb=new Buffer(16)) {
                int returnedStatus=NtFlushBuffersFileEx(directory,0,IntPtr.Zero,0,iosb.Pointer);
                int status=Complete(returnedStatus,directory,iosb);
                stages.Add(Row("stage",stage,"returned_ntstatus",unchecked((uint)returnedStatus),"ntstatus",unchecked((uint)status),"io_status",unchecked((uint)Marshal.ReadInt32(iosb.Pointer)),"io_information",unchecked((ulong)Marshal.ReadInt64(iosb.Pointer,8)),"flush_flags",0));
                int ioStatus=Marshal.ReadInt32(iosb.Pointer);
                if (status<0||ioStatus<0) throw new NativeFailure(stage,status<0?status:ioStatus);
            }
        } else {
            bool ok=FlushFileBuffers(directory); int error=ok?0:Marshal.GetLastWin32Error();
            stages.Add(Row("stage",stage,"win32_error",error));
            if (!ok) throw new NativeFailure(stage,error);
        }
    }
    static int Rename(SafeFileHandle file,SafeFileHandle parent,string name,List<object> stages,string stage) {
        byte[] encoded=Encoding.Unicode.GetBytes(name+"\0"); int offset=Marshal.OffsetOf<RenameHeader>("FirstCharacter").ToInt32();
        int headerSize=Marshal.SizeOf<RenameHeader>();
        using (var data=new Buffer(checked(headerSize+encoded.Length))) {
            Marshal.StructureToPtr(new RenameHeader{Replace=0,Root=parent.DangerousGetHandle(),NameLength=(uint)encoded.Length-2},data.Pointer,false);
            Marshal.Copy(encoded,0,IntPtr.Add(data.Pointer,offset),encoded.Length);
            bool ok=SetFileInformationByHandle(file,3,data.Pointer,(uint)data.Length);
            int error=ok?0:Marshal.GetLastWin32Error();
            stages.Add(Row("stage",stage,"win32_error",error,"rename_form","relative_name_parent_handle","structure_size",headerSize,"filename_offset",offset,"name_byte_count",encoded.Length-2,"encoded_length",encoded.Length,"buffer_length",data.Length));
            return error;
        }
    }
    static Dictionary<string,object> Candidate(string root,string anchorPath,string sid,bool native) {
        var stages=new List<object>(); var result=Row("candidate",native?"nt_directory_flush":"win32_directory_flush","complete",false,"stages",stages);
        string childName=native?"native":"win32",childPath=Path.Combine(root,childName);
        try {
            using (var anchor=OpenDirectory(anchorPath,null,null,native,stages,"open_anchor"))
            using (var parent=OpenDirectory(root,null,null,native,stages,"open_root")) {
                var anchorId=Identity(anchor,true); var parentId=Identity(parent,true);
                Check(anchorId["volume_id"].Equals(parentId["volume_id"]),"cross_volume_root");
                result["filesystem"]=LocalNtfs(anchor); result["root_identity"]=parentId;
                CreateDirectory(childPath,sid); stages.Add(Row("stage","create_child","win32_error",0));
                using (var child=OpenDirectory(childPath,parent,childName,native,stages,"open_child")) {
                    var childId=Identity(child,true); Check(parentId["volume_id"].Equals(childId["volume_id"]),"cross_volume_child");
                    result["child_identity"]=childId;
                    using (var file=CreatePublicFile(Path.Combine(childPath,"temporary"),sid)) {
                        var original=Identity(file,false); result["original_identity"]=original;
                        WritePublic(file); Require(FlushFileBuffers(file),"file_flush_before_rename"); stages.Add(Row("stage","file_flush_before_rename","win32_error",0));
                        int rename=Rename(file,child,"final",stages,"rename_no_replace"); if (rename!=0) throw new NativeFailure("rename_no_replace",rename);
                        Require(FlushFileBuffers(file),"file_flush_after_rename"); stages.Add(Row("stage","file_flush_after_rename","win32_error",0));
                        SameIdentity(original,Identity(file,false)); ReadPublic(file);
                        using (var duplicate=CreatePublicFile(Path.Combine(childPath,"duplicate"),sid)) {
                            var duplicateId=Identity(duplicate,false); WritePublic(duplicate);
                            int collision=Rename(duplicate,child,"final",stages,"duplicate_rename");
                            if (collision!=80&&collision!=183) throw new NativeFailure("not_an_existing_name_collision",collision);
                            SameIdentity(original,Identity(file,false)); ReadPublic(file); SameIdentity(duplicateId,Identity(duplicate,false)); ReadPublic(duplicate);
                            using (var remaining=OpenWin32(Path.Combine(childPath,"duplicate"),Read|ReadControl,OpenReparse,7)) { SameIdentity(duplicateId,Identity(remaining,false)); ReadPublic(remaining); }
                        }
                        DirectoryFlush(child,native,stages,"flush_child");
                        DirectoryFlush(parent,native,stages,"flush_root");
                        DirectoryFlush(anchor,native,stages,"flush_anchor");
                        SameIdentity(original,Identity(file,false)); ReadPublic(file);
                        using (var reopened=OpenWin32(Path.Combine(childPath,"final"),Read|ReadControl,OpenReparse,7)) { SameIdentity(original,Identity(reopened,false)); ReadPublic(reopened); }
                        result["final_identity"]=Identity(file,false); result["exact_public_bytes"]=true; result["collision_preserved_both_originals"]=true;
                    }
                }
            }
            result["complete"]=true;
        } catch (NativeFailure failure) { result["failed_stage"]=failure.Stage; result["error_code"]=failure.Code; }
        catch (EntryPointNotFoundException failure) when (native) { result["failed_stage"]="candidate_native_entrypoint_unavailable"; result["error_code"]=failure.HResult; }
        catch (DllNotFoundException failure) when (native) { result["failed_stage"]="candidate_native_library_unavailable"; result["error_code"]=failure.HResult; }
        return result;
    }
    public static Dictionary<string,object> Run(string anchorPath) {
        var report=Row("schema",1,"scope","fixed_local_ntfs_compatibility_only","power_loss_qualified",false,"complete",false,"candidates",new List<object>());
        string suffix=Guid.NewGuid().ToString("N"),root=Path.Combine(anchorPath,"namespace-probe-"+suffix),adminControl=Path.Combine(anchorPath,"namespace-admin-control-"+suffix);
        bool impersonating=false,rootCreated=false,adminCreated=false;
        try {
            Check(IntPtr.Size==8,"requires_native_x64");
            Require(OpenProcessToken(GetCurrentProcess(),0x8a,out SafeFileHandle source),"open_process_token");
            using (source) using (var token=Restricted(source)) {
                string sid=User(source); report["source_token"]=TokenReport(source,false);
                bool sourceAdmin; using (var g=TokenInfo(source,2)) sourceAdmin=Groups(g).Any(group=>SidText(group.Sid)==Admin&&(group.Attributes&Enabled)!=0);
                if (sourceAdmin) {
                    using (var control=CreatePublicFile(adminControl,sid,true)) { adminCreated=true; WritePublic(control); }
                    using (var control=OpenWin32(adminControl,Read,OpenReparse)) ReadPublic(control);
                    report["original_admin_control_read"]=true;
                }
                Require(ImpersonateLoggedOnUser(token),"impersonate_restricted_token"); impersonating=true;
                Require(OpenThreadToken(GetCurrentThread(),8,true,out SafeFileHandle effective),"open_effective_thread_token");
                using (effective) { Check(User(effective)==sid,"effective_user_changed"); report["effective_token"]=TokenReport(effective,true); report["ordinary_rights_verified"]=true; }
                if (sourceAdmin) {
                    using (var denied=CreateFileW(adminControl,Read,ShareReadWrite,IntPtr.Zero,3,OpenReparse,IntPtr.Zero)) {
                        int code=denied.IsInvalid?Marshal.GetLastWin32Error():0;
                        report["restricted_admin_control_error"]=code; Check(denied.IsInvalid&&code==5,"admin_control_not_denied");
                    }
                } else report["admin_contrast"]="not_applicable_source_admin_group_not_enabled";
                CreateDirectory(root,sid); rootCreated=true;
                using (var positive=CreatePublicFile(Path.Combine(root,"owner-positive"),sid)) WritePublic(positive);
                using (var positive=OpenWin32(Path.Combine(root,"owner-positive"),Read,OpenReparse)) ReadPublic(positive);
                report["restricted_owner_control_read"]=true;
                var candidates=(List<object>)report["candidates"];
                var a=Candidate(root,anchorPath,sid,false); candidates.Add(a);
                var b=Candidate(root,anchorPath,sid,true); candidates.Add(b);
                report["complete"]=((bool)a["complete"])||((bool)b["complete"]);
            }
        } catch (NativeFailure failure) { report["failed_stage"]=failure.Stage; report["error_code"]=failure.Code; }
        catch (DllNotFoundException failure) { report["failed_stage"]="native_library_unavailable"; report["error_code"]=failure.HResult; report["complete"]=false; }
        catch (EntryPointNotFoundException failure) { report["failed_stage"]="native_entrypoint_unavailable"; report["error_code"]=failure.HResult; report["complete"]=false; }
        catch (MarshalDirectiveException failure) { report["failed_stage"]="native_marshalling_failed"; report["error_code"]=failure.HResult; report["complete"]=false; }
        catch (OverflowException failure) { report["failed_stage"]="native_extent_overflow"; report["error_code"]=failure.HResult; report["complete"]=false; }
        catch { report["failed_stage"]="managed_probe_failure"; report["complete"]=false; }
        finally {
            if (impersonating&&!RevertToSelf()) Abort("thread_token_reversion_failed",unchecked((uint)Marshal.GetLastWin32Error()));
            try { if (adminCreated) File.Delete(adminControl); if (rootCreated) Directory.Delete(root,true); report["owned_fixture_cleanup"]=true; }
            catch { report["owned_fixture_cleanup"]=false; report["complete"]=false; }
        }
        return report;
    }
}
